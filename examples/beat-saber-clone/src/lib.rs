mod components;
mod systems;

use crate::components::cube::Colour;
use crate::systems::cube_spawner::{create_cubes, cube_spawner_system};
use cube::Cube;
use legion::IntoQuery;
use std::collections::HashMap;

use hotham::components::hand::Handedness;
use hotham::components::{Mesh, RigidBody, Transform, TransformMatrix};
use hotham::gltf_loader::add_model_to_world;
use hotham::legion::{Resources, Schedule, World};
use hotham::resources::{PhysicsContext, RenderContext, XrContext};
use hotham::schedule_functions::{begin_frame, end_frame, physics_step};
use hotham::systems::rendering::rendering_system;
use hotham::systems::{
    collision_system, update_parent_transform_matrix_system, update_rigid_body_transforms_system,
    update_transform_matrix_system,
};
use hotham::{gltf_loader, App, HothamResult};
use hotham_debug_server::DebugServer as DebugServerT;

use nalgebra::{Isometry, Matrix4, Vector3};
use serde::{Deserialize, Serialize};
use systems::sabers::{add_saber_physics, sabers_system};

use crate::components::{cube, Saber};

#[cfg_attr(target_os = "android", ndk_glue::main(backtrace = "on"))]
pub fn main() {
    println!("[BEAT_SABER_EXAMPLE] MAIN!");
    real_main().unwrap();
}

type EditableData = ();
type NonEditableData = Vec<DebugInfo>;
type DebugServer = DebugServerT<EditableData, NonEditableData>;
type Models = HashMap<String, World>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DebugInfo {
    position: Vector3<f32>,
    translation: Matrix4<f32>,
}

pub fn real_main() -> HothamResult<()> {
    let (xr_context, vulkan_context) = XrContext::new()?;
    let render_context = RenderContext::new(&vulkan_context, &xr_context)?;
    let mut physics_context = PhysicsContext::default();
    let mut world = World::default();
    let glb_bufs: Vec<&[u8]> = vec![include_bytes!("../assets/beat_saber.glb")];
    let models: Models = gltf_loader::load_models_from_glb(
        &glb_bufs,
        &vulkan_context,
        &render_context.descriptor_set_layouts,
    )?;

    // Add Environment
    add_environment_models(&models, &mut world, &vulkan_context, &render_context);

    // Add cubes
    let (red_cubes, blue_cubes) = add_cubes(
        10,
        &models,
        &mut world,
        &mut physics_context,
        &vulkan_context,
        &render_context,
    );

    // Add Red Saber
    add_saber(
        Colour::Red,
        &models,
        &mut world,
        &vulkan_context,
        &render_context,
        &mut physics_context,
    );

    // Add Blue Saber
    add_saber(
        Colour::Blue,
        &models,
        &mut world,
        &vulkan_context,
        &render_context,
        &mut physics_context,
    );

    let debug_server = DebugServer::new();

    let mut resources = Resources::default();
    resources.insert(xr_context);
    resources.insert(vulkan_context);
    resources.insert(physics_context);
    resources.insert(debug_server);
    resources.insert(render_context);
    resources.insert(models);
    resources.insert(0 as usize);

    let schedule = Schedule::builder()
        .add_thread_local_fn(begin_frame)
        .add_system(collision_system())
        .add_thread_local_fn(physics_step)
        .add_system(sabers_system())
        .add_system(cube_spawner_system(red_cubes, blue_cubes, 0))
        .add_system(update_rigid_body_transforms_system())
        .add_system(update_transform_matrix_system())
        .add_system(update_parent_transform_matrix_system())
        .add_thread_local_fn(move |world, r| {
            let mut debug_server = r.get_mut::<DebugServer>().unwrap();
            let physics_context = r.get::<PhysicsContext>().unwrap();
            let mut query = <(&Mesh, &RigidBody, &Cube, &TransformMatrix)>::query();
            let renderable_objects = query
                .iter(world)
                .map(|(_, r, _, t)| {
                    let rigid_body = &physics_context.rigid_bodies[r.handle];
                    DebugInfo {
                        position: rigid_body.position().translation.vector,
                        translation: t.0,
                    }
                })
                .collect::<Vec<_>>();
            if let Some(_) = debug_server.sync(&renderable_objects) {
                // let mut entity = world.entry_mut(environment).unwrap();
                // *entity.get_component_mut::<Transform>().unwrap() = updated.environment;
                // let mut entity = world.entry_mut(ramp).unwrap();
                // *entity.get_component_mut::<Transform>().unwrap() = updated.ramp;
                // render_context
                //     .scene_params_buffer
                //     .update(&vulkan_context, &[updated])
                //     .expect("Unable to update data");
            };
        })
        .add_system(rendering_system())
        .add_thread_local_fn(end_frame)
        .build();

    let mut app = App::new(world, resources, schedule)?;
    app.run()?;
    Ok(())
}

fn add_saber(
    colour: Colour,
    models: &HashMap<String, World>,
    world: &mut World,
    vulkan_context: &hotham::resources::vulkan_context::VulkanContext,
    render_context: &RenderContext,
    physics_context: &mut PhysicsContext,
) {
    let (model_name, handedness) = match colour {
        Colour::Red => ("Red Saber", Handedness::Left),
        Colour::Blue => ("Blue Saber", Handedness::Right),
    };
    let saber = add_model_to_world(
        model_name,
        models,
        world,
        None,
        vulkan_context,
        &render_context.descriptor_set_layouts,
    )
    .unwrap();
    {
        add_saber_physics(world, physics_context, saber);
        let mut saber_entry = world.entry(saber).unwrap();
        saber_entry.add_component(Saber { handedness });
    }
}

fn add_cubes(
    cube_count: usize,
    models: &HashMap<String, World>,
    world: &mut World,
    physics_context: &mut PhysicsContext,
    vulkan_context: &hotham::resources::vulkan_context::VulkanContext,
    render_context: &RenderContext,
) -> (Vec<legion::Entity>, Vec<legion::Entity>) {
    let red_cubes = create_cubes(
        cube_count,
        cube::Colour::Red,
        models,
        world,
        physics_context,
        vulkan_context,
        &render_context.descriptor_set_layouts,
    );
    let blue_cubes = create_cubes(
        cube_count,
        cube::Colour::Blue,
        models,
        world,
        physics_context,
        vulkan_context,
        &render_context.descriptor_set_layouts,
    );
    (red_cubes, blue_cubes)
}

fn add_environment_models(
    models: &HashMap<String, World>,
    world: &mut World,
    vulkan_context: &hotham::resources::vulkan_context::VulkanContext,
    render_context: &RenderContext,
) {
    let _environment = add_model_to_world(
        "Environment",
        models,
        world,
        None,
        vulkan_context,
        &render_context.descriptor_set_layouts,
    )
    .unwrap();
    let _ramp = add_model_to_world(
        "Ramp",
        models,
        world,
        None,
        vulkan_context,
        &render_context.descriptor_set_layouts,
    )
    .unwrap();
}
