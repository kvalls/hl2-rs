//! Runtime model colliders enter entity queries before the next physics step.
use super::{native_prop_world, Physics, Vec3};
use modkit_core::{Entity, PhysicsSolid};

#[test]
fn runtime_pickup_publishes_all_native_pieces_before_the_physics_step() {
    let mut world = native_prop_world();
    let mut instance = world.model_instances.pop().expect("synthetic model");
    instance.kind = "item_battery".into();
    instance.entity = Some(0);
    world.entities.push(Entity {
        properties: vec![("classname".into(), instance.kind.clone())],
    });
    world.model_physics.insert(
        instance.asset_key(),
        PhysicsSolid {
            mass: 3.,
            ..Default::default()
        },
    );
    let mut physics = Physics::new(&world);
    physics.add_spawned_instance(&world, &instance);
    let pieces = &physics.entity_colliders[&0];
    assert_eq!(pieces.len(), 2);
    assert_eq!(&physics.changed_entity_colliders, pieces);
    assert!((physics.entity_mass(0).unwrap() - 3.).abs() < 0.001);

    let shoot = |physics: &Physics, offset| {
        physics.impact_ray(
            instance.origin + Vec3::new(0., offset, 100.),
            -Vec3::Z,
            200.,
        )
    };
    assert!(shoot(&physics, 12.).is_none());
    physics.refresh_entity_queries();
    for offset in [-12., 12.] {
        let hit = shoot(&physics, offset).expect("published native piece");
        assert_eq!(hit.entity, 0);
    }
    assert!(
        shoot(&physics, 0.).is_none(),
        "the gap between pieces stays open"
    );
}
