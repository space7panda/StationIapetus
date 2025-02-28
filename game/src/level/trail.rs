use fyrox::core::sstorage::ImmutableString;
use fyrox::material::PropertyValue;
use fyrox::scene::mesh::Mesh;
use fyrox::scene::sprite::Sprite;
use fyrox::utils::log::Log;
use fyrox::{
    core::{pool::Handle, visitor::prelude::*, VecExtensions},
    scene::{node::Node, Scene},
};

#[derive(Default, Visit)]
pub struct ShotTrail {
    node: Handle<Node>,
    lifetime: f32,
    max_lifetime: f32,
}

impl ShotTrail {
    pub fn new(node: Handle<Node>, max_lifetime: f32) -> Self {
        Self {
            node,
            lifetime: 0.0,
            max_lifetime,
        }
    }
}

#[derive(Default, Visit)]
pub struct ShotTrailContainer {
    container: Vec<ShotTrail>,
}

impl ShotTrailContainer {
    pub fn update(&mut self, dt: f32, scene: &mut Scene) {
        self.container.retain_mut_ext(|trail| {
            trail.lifetime = (trail.lifetime + dt).min(trail.max_lifetime);
            let k = 1.0 - trail.lifetime / trail.max_lifetime;
            let new_alpha = (255.0 * k) as u8;

            let trait_node = &mut scene.graph[trail.node];
            if let Some(mesh) = trait_node.cast_mut::<Mesh>() {
                for surface in mesh.surfaces_mut() {
                    let mut material = surface.material().lock();
                    let color = material
                        .property_ref(&ImmutableString::new("diffuseColor"))
                        .unwrap()
                        .as_color()
                        .unwrap();
                    Log::verify(material.set_property(
                        &ImmutableString::new("diffuseColor"),
                        PropertyValue::Color(color.with_new_alpha(new_alpha)),
                    ));
                }
            } else if let Some(sprite) = trait_node.cast_mut::<Sprite>() {
                sprite.set_color(sprite.color().with_new_alpha(new_alpha));
            }

            if trail.lifetime >= trail.max_lifetime {
                scene.remove_node(trail.node);
            }
            trail.lifetime < trail.max_lifetime
        });
    }

    pub fn add(&mut self, trail: ShotTrail) {
        self.container.push(trail);
    }
}
