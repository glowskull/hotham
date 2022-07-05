use hecs::{PreparedQuery, World};
use render_context::RenderContext;

use crate::{
    components::{Skin, TransformMatrix},
    resources::render_context,
};

/// Skinning system
/// Walks through each joint in the system and builds up the `joint_matrices` that will be sent to the vertex shader
pub fn skinning_system(
    skins_query: &mut PreparedQuery<(&Skin, &TransformMatrix)>,
    world: &mut World,
    render_context: &mut RenderContext,
) {
    for (_, (skin, transform_matrix)) in skins_query.query(world).iter() {
        let buffer = unsafe { render_context.resources.skins_buffer.as_slice_mut() };
        let joint_matrices = &mut buffer[skin.id as usize];
        let inverse_transform = transform_matrix.0.try_inverse().unwrap();

        for (n, (joint, inverse_bind_matrix)) in skin
            .joints
            .iter()
            .zip(skin.inverse_bind_matrices.iter())
            .enumerate()
        {
            let joint_transform = world.get::<TransformMatrix>(*joint).unwrap().0;
            let joint_matrix = inverse_transform * joint_transform * inverse_bind_matrix;
            joint_matrices[n] = joint_matrix;
        }
    }
}

#[cfg(target_os = "windows")]
#[cfg(test)]
mod tests {

    use std::io::Write;

    use crate::{
        components::{Info, Skin},
        util::get_world_with_hands,
    };

    use super::*;
    use approx::relative_eq;
    use nalgebra::Matrix4;

    #[test]
    pub fn test_hand_skinning() {
        let (mut render_context, vulkan_context) = RenderContext::testing();
        let mut world = get_world_with_hands(&vulkan_context, &mut render_context);
        skinning_system(&mut Default::default(), &mut world, &mut render_context);

        assert!(verify_matrices(&world, &render_context));

        // Muck all the joints up
        for (_, skin) in world.query::<&Skin>().iter() {
            for joint in &skin.joints {
                let mut transform_matrix = world.get_mut::<TransformMatrix>(*joint).unwrap();
                transform_matrix.0 = Matrix4::zeros();
            }
        }
        skinning_system(&mut Default::default(), &mut world, &mut render_context);

        assert!(!verify_matrices(&world, &render_context));
    }

    fn verify_matrices(world: &World, render_context: &RenderContext) -> bool {
        let mut called = 0;
        for (_, (skin, info)) in world.query::<(&Skin, &Info)>().iter() {
            let correct_matrices: Vec<Matrix4<f32>> = if info.name == "hands:Lhand" {
                println!("Left hand!");
                serde_json::from_slice(include_bytes!(
                    "../../../test_assets/left_hand_skinned_matrices.json"
                ))
                .unwrap()
            } else {
                println!("Right hand!");
                serde_json::from_slice(include_bytes!(
                    "../../../test_assets/right_hand_skinned_matrices.json"
                ))
                .unwrap()
            };
            let buffer = unsafe { render_context.resources.skins_buffer.as_slice() };
            let joint_matrices = &buffer[skin.id as usize];

            for i in 0..correct_matrices.len() {
                let expected = correct_matrices[i];
                let actual = joint_matrices[i];
                if !relative_eq!(expected, actual) {
                    println!("Matrix {} is incorrect", i);
                    println!("Actual:");
                    println!("{}", serde_json::to_string_pretty(&actual).unwrap());
                    println!("Expected:");
                    println!("{}", serde_json::to_string_pretty(&expected).unwrap());
                    std::fs::File::create("matrix_failed.json")
                        .unwrap()
                        .write_all(&serde_json::to_vec_pretty(&joint_matrices[..]).unwrap())
                        .unwrap();
                    return false;
                }
            }
            called += 1;
        }
        assert_ne!(called, 0);

        true
    }
}
