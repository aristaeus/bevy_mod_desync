# bevy_mod_desync
A small proof of concept desync tracker for Bevy.

## What? Why?
This provides a way of tracking whether determinism is achieved when running a Bevy app on different architectures or at different times. For example, in a game with deterministic lockstep multiplayer, clients can regularly check whether they agree on the total state of the Bevy world.

## How?
This crate provides a bevy `Plugin` which creates a resource and adds a single system. The resource, `Crc`, contains the [Cyclic Redundancy Check](https://en.wikipedia.org/wiki/Cyclic_redundancy_check) of the Bevy `World`. This is updated at the start of every tick. It is up to you to check whether this hash matches what you expect. Entities must be marked for desync tracking with the `TrackDesync` component. Components must be registered for tracking with `app.track_desync::<C>()`, and implement `Serialize`

### Usage
Taken from the crate tests. This demonstrates using a custom `EntityMapper` for sorting entities to prevent false positives.
```rust
fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(DesyncPlugin::default())
        .track_desync::<Foo>();
    app
}

#[derive(Clone, Default, Resource)]
struct EntityMap {
    entity_map: bevy_ecs::entity::EntityHashMap<Entity>,
}

// other trait impls...

fn entity_mapping_sync_and_desync() {
    let mut app_1 = build_app();
    let mut app_2 = build_app();
    let foo_1_0 = app_1.world.spawn((Foo(0), TrackDesync)).id();
    let foo_1_1 = app_1.world.spawn((Foo(1), TrackDesync)).id();
    let foo_2_1 = app_2.world.spawn((Foo(1), TrackDesync)).id();
    let foo_2_0 = app_2.world.spawn((Foo(0), TrackDesync)).id();

    // calculate crc
    app_1.update();
    app_2.update();

    // because entities were spawned in a different order, these checksums don't match
    assert_ne!(app_1.world.resource::<Crc>(), app_2.world.resource::<Crc>());
    let mut entity_map = bevy_ecs::entity::EntityHashMap::default();
    entity_map.insert(foo_1_0, foo_2_0);
    entity_map.insert(foo_1_1, foo_2_1);

    // switch to using the entity map instead
    app_1.world.insert_resource(EntityMap {
        entity_map: entity_map.clone(),
    });
    app_1.world.resource_mut::<DesyncPluginData>().entity_sort =
        Arc::new(Box::new(|w| sort_from_entity_map::<EntityMap>(w, true)));
    app_2.world.insert_resource(EntityMap {
        entity_map: entity_map.clone(),
    });
    app_2.world.resource_mut::<DesyncPluginData>().entity_sort =
        Arc::new(Box::new(|w| sort_from_entity_map::<EntityMap>(w, false)));

    // checksums now match
    app_1.update();
    app_2.update();
    assert_eq!(app_1.world.resource::<Crc>(), app_2.world.resource::<Crc>());

    // oh no, desync!
    *app_1.world.get_mut::<Foo>(foo_1_0).unwrap() = Foo(2);

    app_1.update();
    app_2.update();
    assert_ne!(app_1.world.resource::<Crc>(), app_2.world.resource::<Crc>());
}
```

### Caveats
This has not yet been tested in a real world app, and will likely trigger a lot of false positives. In an attempt to make hashing deterministic, archetypes, entities and components are sorted by ID before serialization - however I don't think Bevy offers any guarantees on determinism there. Users with a way to enforce entity sorting (e.g. a multiplayer game with a complete `EntityMapper` can provide a `fn(&World) -> Vec<Entity>` that has a canonical ordering for full determinism.

## Open Questions
* Is `First` early enough in the schedule? How can we best guarantee the hashing runs at the same time every frame?
* How do we handle dynamic types? Something involving `bevy_reflect`?
* There's a couple of manual steps (register components, mark entities for tracking). Can we make this more automatic, and if so, without causing too many false positives?
* Should we take in a "callback" at plugin creation instead of expecting users to 
* Can we stop false positives!
