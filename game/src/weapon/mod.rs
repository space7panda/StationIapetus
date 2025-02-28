//! Weapon related stuff.

use crate::{
    actor::{Actor, ActorContainer},
    character::HitBox,
    message::Message,
    weapon::{
        definition::{WeaponDefinition, WeaponKind, WeaponProjectile},
        projectile::Shooter,
        sight::LaserSight,
    },
    CollisionGroups, GameTime, MessageSender,
};
use fyrox::scene::collider::BitMask;
use fyrox::{
    core::{
        algebra::{Matrix3, Point3, Vector3},
        color::Color,
        math::{ray::Ray, Matrix4Ext},
        pool::{Handle, Pool},
        sstorage::ImmutableString,
        visitor::{Visit, VisitResult, Visitor},
    },
    engine::resource_manager::ResourceManager,
    material::{shader::SamplerFallback, PropertyValue},
    rand::seq::SliceRandom,
    scene::{
        base::BaseBuilder,
        collider::InteractionGroups,
        graph::{
            physics::{FeatureId, Intersection, PhysicsWorld, RayCastOptions},
            Graph,
        },
        light::{point::PointLightBuilder, spot::SpotLightBuilder, BaseLightBuilder},
        mesh::RenderPath,
        node::Node,
        Scene,
    },
    utils::{
        self,
        log::{Log, MessageKind},
    },
};
use std::{
    hash::{Hash, Hasher},
    ops::{Index, IndexMut},
    path::PathBuf,
};

pub mod definition;
pub mod projectile;
pub mod sight;

#[derive(Visit)]
pub struct Weapon {
    kind: WeaponKind,
    model: Handle<Node>,
    shot_point: Handle<Node>,
    muzzle_flash: Handle<Node>,
    shot_light: Handle<Node>,
    last_shot_time: f64,
    shot_position: Vector3<f32>,
    owner: Handle<Actor>,
    muzzle_flash_timer: f32,
    #[visit(skip)]
    pub definition: &'static WeaponDefinition,
    flash_light: Handle<Node>,
    laser_sight: LaserSight,
}

#[derive(Clone)]
pub struct Hit {
    pub actor: Handle<Actor>, // Can be None if level geometry was hit.
    pub who: Handle<Actor>,
    pub position: Vector3<f32>,
    pub normal: Vector3<f32>,
    pub collider: Handle<Node>,
    pub feature: FeatureId,
    pub hit_box: Option<HitBox>,
    pub query_buffer: Vec<Intersection>,
}

impl PartialEq for Hit {
    fn eq(&self, other: &Self) -> bool {
        self.actor == other.actor
            && self.who == other.who
            && self.position == other.position
            && self.normal == other.normal
            && self.collider == other.collider
            && self.feature == other.feature
            && self.hit_box == other.hit_box
    }
}

impl Hash for Hit {
    fn hash<H: Hasher>(&self, state: &mut H) {
        utils::hash_as_bytes(self, state);
    }
}

impl Eq for Hit {}

/// Checks intersection of given ray with actors and environment.
pub fn ray_hit(
    begin: Vector3<f32>,
    end: Vector3<f32>,
    shooter: Shooter,
    weapons: &WeaponContainer,
    actors: &ActorContainer,
    physics: &mut PhysicsWorld,
    ignored_collider: Handle<Node>,
) -> Option<Hit> {
    let ray = Ray::from_two_points(begin, end);

    // TODO: Avoid allocation.
    let mut query_buffer = Vec::default();

    physics.cast_ray(
        RayCastOptions {
            ray_origin: Point3::from(ray.origin),
            ray_direction: ray.dir,
            max_len: ray.dir.norm(),
            groups: InteractionGroups::new(
                BitMask(0xFFFF),
                BitMask(!(CollisionGroups::ActorCapsule as u32)),
            ),
            sort_results: true,
        },
        &mut query_buffer,
    );

    // List of hits sorted by distance from ray origin.
    if let Some(hit) = query_buffer.iter().find(|i| i.collider != ignored_collider) {
        let mut is_hitbox_hit = false;

        // Check if there was an intersection with an actor.
        'actor_loop: for (actor_handle, actor) in actors.pair_iter() {
            for hit_box in actor.hit_boxes.iter() {
                if hit_box.collider == hit.collider {
                    is_hitbox_hit = true;

                    let who = match shooter {
                        Shooter::None | Shooter::Turret(_) => Default::default(),
                        Shooter::Actor(actor) => actor,
                        Shooter::Weapon(weapon) => weapons[weapon].owner(),
                    };

                    // Ignore intersections with owners.
                    if who == actor_handle {
                        continue 'actor_loop;
                    }

                    return Some(Hit {
                        actor: actor_handle,
                        who,
                        position: hit.position.coords,
                        normal: hit.normal,
                        collider: hit.collider,
                        feature: hit.feature,
                        hit_box: Some(*hit_box),
                        query_buffer,
                    });
                }
            }
        }

        if is_hitbox_hit {
            None
        } else {
            Some(Hit {
                actor: Handle::NONE,
                who: Handle::NONE,
                position: hit.position.coords,
                normal: hit.normal,
                collider: hit.collider,
                feature: hit.feature,
                hit_box: None,
                query_buffer,
            })
        }
    } else {
        None
    }
}

impl Default for Weapon {
    fn default() -> Self {
        Self {
            kind: WeaponKind::M4,
            model: Handle::NONE,
            shot_point: Handle::NONE,
            last_shot_time: 0.0,
            shot_position: Vector3::default(),
            owner: Handle::NONE,
            muzzle_flash_timer: 0.0,
            definition: Self::definition(WeaponKind::M4),
            muzzle_flash: Default::default(),
            shot_light: Default::default(),
            flash_light: Default::default(),
            laser_sight: Default::default(),
        }
    }
}

impl Weapon {
    pub fn definition(kind: WeaponKind) -> &'static WeaponDefinition {
        definition::DEFINITIONS.map.get(&kind).unwrap()
    }

    pub async fn new(
        kind: WeaponKind,
        resource_manager: ResourceManager,
        scene: &mut Scene,
    ) -> Weapon {
        let definition = Self::definition(kind);

        let model = resource_manager
            .request_model(&definition.model)
            .await
            .unwrap()
            .instantiate_geometry(scene);

        let shot_point = scene.graph.find_by_name(model, "Weapon:ShotPoint");

        if shot_point.is_none() {
            Log::writeln(
                MessageKind::Warning,
                format!("Shot point not found for {:?} weapon!", kind),
            );
        }

        let muzzle_flash = scene.graph.find_by_name(model, "MuzzleFlash");

        let shot_light = if muzzle_flash.is_none() {
            Log::writeln(
                MessageKind::Warning,
                format!("Muzzle flash not found for {:?} weapon!", kind),
            );
            Default::default()
        } else {
            let light = PointLightBuilder::new(
                BaseLightBuilder::new(BaseBuilder::new().with_visibility(false))
                    .with_scatter_enabled(false)
                    .with_color(Color::opaque(255, 255, 255)),
            )
            .with_radius(2.0)
            .build(&mut scene.graph);

            scene.graph.link_nodes(light, muzzle_flash);

            // Explicitly define render path to be able to render transparent muzzle flash.
            scene.graph[muzzle_flash]
                .as_mesh_mut()
                .set_render_path(RenderPath::Forward);

            light
        };

        let flash_light_point = scene.graph.find_by_name(model, "FlashLightPoint");

        let flash_light = if flash_light_point.is_some() {
            let flash_light = SpotLightBuilder::new(
                BaseLightBuilder::new(BaseBuilder::new())
                    .with_scatter_enabled(true)
                    .with_scatter_factor(Vector3::new(0.1, 0.1, 0.1)),
            )
            .with_distance(10.0)
            .with_cookie_texture(resource_manager.request_texture("data/particles/light_01.png"))
            .with_hotspot_cone_angle(30.0f32.to_radians())
            .build(&mut scene.graph);

            scene.graph.link_nodes(flash_light, flash_light_point);

            flash_light
        } else {
            Handle::NONE
        };

        Weapon {
            kind,
            model,
            shot_point,
            definition,
            muzzle_flash,
            shot_light,
            flash_light,
            laser_sight: LaserSight::new(scene, resource_manager),
            ..Default::default()
        }
    }

    pub fn set_visibility(&self, visibility: bool, graph: &mut Graph) {
        graph[self.model].set_visibility(visibility);
        if !visibility {
            self.laser_sight.set_visible(visibility, graph);
        }
    }

    pub fn model(&self) -> Handle<Node> {
        self.model
    }

    pub fn update(&mut self, scene: &mut Scene, actors: &ActorContainer, dt: f32) {
        let node = &mut scene.graph[self.model];
        self.shot_position = node.global_position();

        self.muzzle_flash_timer -= dt;
        if self.muzzle_flash_timer <= 0.0 && self.muzzle_flash.is_some() {
            scene.graph[self.muzzle_flash].set_visibility(false);
            scene.graph[self.shot_light].set_visibility(false);
        }

        let mut ignored_collider = Default::default();
        if actors.contains(self.owner) {
            ignored_collider = actors.get(self.owner).capsule_collider;
        }

        let dir = self.shot_direction(&scene.graph);
        let pos = self.shot_position(&scene.graph);
        self.laser_sight
            .update(scene, pos, dir, ignored_collider, dt)
    }

    pub fn shot_position(&self, graph: &Graph) -> Vector3<f32> {
        if self.shot_point.is_some() {
            graph[self.shot_point].global_position()
        } else {
            // Fallback
            graph[self.model].global_position()
        }
    }

    pub fn shot_direction(&self, graph: &Graph) -> Vector3<f32> {
        graph[self.model].look_vector().normalize()
    }

    pub fn kind(&self) -> WeaponKind {
        self.kind
    }

    pub fn world_basis(&self, graph: &Graph) -> Matrix3<f32> {
        graph[self.model].global_transform().basis()
    }

    pub fn owner(&self) -> Handle<Actor> {
        self.owner
    }

    pub fn set_owner(&mut self, owner: Handle<Actor>) {
        self.owner = owner;
    }

    pub fn switch_flash_light(&self, graph: &mut Graph) {
        if self.flash_light.is_some() {
            let flash_light = &mut graph[self.flash_light];
            let enabled = flash_light.visibility();
            flash_light.set_visibility(!enabled);
        }
    }

    pub fn laser_sight(&self) -> &LaserSight {
        &self.laser_sight
    }

    pub fn laser_sight_mut(&mut self) -> &mut LaserSight {
        &mut self.laser_sight
    }

    pub fn can_shoot(&self, time: GameTime) -> bool {
        time.elapsed - self.last_shot_time >= self.definition.shoot_interval
    }

    pub fn shoot(
        &mut self,
        self_handle: Handle<Weapon>,
        scene: &mut Scene,
        time: GameTime,
        resource_manager: ResourceManager,
        direction: Option<Vector3<f32>>,
        sender: &MessageSender,
    ) {
        self.last_shot_time = time.elapsed;

        let position = self.shot_position(&scene.graph);

        if let Some(random_shot_sound) = self
            .definition
            .shot_sounds
            .choose(&mut fyrox::rand::thread_rng())
        {
            sender.send(Message::PlaySound {
                path: PathBuf::from(random_shot_sound.clone()),
                position,
                gain: 1.0,
                rolloff_factor: 5.0,
                radius: 3.0,
            });
        }

        if self.muzzle_flash.is_some() {
            let muzzle_flash = &mut scene.graph[self.muzzle_flash];
            muzzle_flash.set_visibility(true);
            for surface in muzzle_flash.as_mesh_mut().surfaces_mut() {
                let textures = [
                    "data/particles/muzzle_01.png",
                    "data/particles/muzzle_02.png",
                    "data/particles/muzzle_03.png",
                    "data/particles/muzzle_04.png",
                    "data/particles/muzzle_05.png",
                ];
                Log::verify(surface.material().lock().set_property(
                    &ImmutableString::new("diffuseTexture"),
                    PropertyValue::Sampler {
                        value: Some(resource_manager.request_texture(
                            textures.choose(&mut fyrox::rand::thread_rng()).unwrap(),
                        )),
                        fallback: SamplerFallback::White,
                    },
                ));
            }
            scene.graph[self.shot_light].set_visibility(true);
            self.muzzle_flash_timer = 0.075;
        }

        let position = self.shot_position(&scene.graph);
        let direction = direction
            .unwrap_or_else(|| self.shot_direction(&scene.graph))
            .try_normalize(std::f32::EPSILON)
            .unwrap_or_else(Vector3::z);

        match self.definition.projectile {
            WeaponProjectile::Projectile(projectile) => sender.send(Message::CreateProjectile {
                kind: projectile,
                position,
                direction,
                shooter: Shooter::Weapon(self_handle),
                initial_velocity: Default::default(),
            }),
            WeaponProjectile::Ray { damage } => {
                sender.send(Message::ShootRay {
                    shooter: Shooter::Weapon(self_handle),
                    begin: position,
                    end: position + direction.scale(1000.0),
                    damage,
                    shot_effect: self.definition.shot_effect,
                });
            }
        }
    }

    pub fn clean_up(&mut self, scene: &mut Scene) {
        scene.graph.remove_node(self.model);
        self.laser_sight.clean_up(scene);
    }

    pub fn resolve(&mut self) {
        self.definition = Self::definition(self.kind);
    }
}

#[derive(Default, Visit)]
pub struct WeaponContainer {
    pool: Pool<Weapon>,
}

impl WeaponContainer {
    pub fn new() -> Self {
        Self { pool: Pool::new() }
    }

    pub fn add(&mut self, weapon: Weapon) -> Handle<Weapon> {
        self.pool.spawn(weapon)
    }

    pub fn try_get(&self, weapon: Handle<Weapon>) -> Option<&Weapon> {
        self.pool.try_borrow(weapon)
    }

    pub fn contains(&self, weapon: Handle<Weapon>) -> bool {
        self.pool.is_valid_handle(weapon)
    }

    pub fn free(&mut self, weapon: Handle<Weapon>) {
        self.pool.free(weapon);
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Weapon> {
        self.pool.iter_mut()
    }

    pub fn update(&mut self, scene: &mut Scene, actors: &ActorContainer, dt: f32) {
        for weapon in self.pool.iter_mut() {
            weapon.update(scene, actors, dt)
        }
    }

    pub fn resolve(&mut self) {
        for weapon in self.pool.iter_mut() {
            weapon.resolve();
        }
    }
}

impl Index<Handle<Weapon>> for WeaponContainer {
    type Output = Weapon;

    fn index(&self, index: Handle<Weapon>) -> &Self::Output {
        &self.pool[index]
    }
}

impl IndexMut<Handle<Weapon>> for WeaponContainer {
    fn index_mut(&mut self, index: Handle<Weapon>) -> &mut Self::Output {
        &mut self.pool[index]
    }
}
