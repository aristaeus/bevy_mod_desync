# bevy_mod_desync
A small proof of concept desync tracker for Bevy.

## What? Why?
This provides a way of tracking whether determinism is achieved when running a Bevy app on different architectures or at different times. For example, in a game with deterministic lockstep multiplayer, clients can regularly check whether they agree on the total state of the Bevy world.

## How?
This crate provides a bevy `Plugin` which creates a resource and adds a single system. The resource, `Crc`, contains the [Cyclic Redundancy Check](https://en.wikipedia.org/wiki/Cyclic_redundancy_check) of the Bevy `World`. This is updated at the start of every tick. It is up to you to check whether this hash matches what you expect. Entities must be marked for desync tracking with the `TrackDesync` component. Components must be registered for tracking with `app.track_desync::<C>()`, and implement `Serialize`

### Usage
Taken from the crate tests.
```rust
fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(DesyncPlugin::default())
        .track_desync::<Foo>();
    app
}

fn detect_sync() {
    let mut app_1 = build_app();
    let mut app_2 = build_app();
    app_1.world.spawn((Foo(0), TrackDesync));
    app_2.world.spawn((Foo(0), TrackDesync));

    // calculate crc
    app_1.update();
    app_2.update();

    assert_eq!(app_1.world.resource::<Crc>(), app_2.world.resource::<Crc>());
}

fn detect_desync() {
    let mut app_1 = build_app();
    let mut app_2 = build_app();
    app_1.world.spawn((Foo(0), TrackDesync));
    app_2.world.spawn((Foo(1), TrackDesync));

    // calculate crc
    app_1.update();
    app_2.update();

    assert_ne!(app_1.world.resource::<Crc>(), app_2.world.resource::<Crc>());
}
```

### Caveats
This has not yet been tested in a real world app, and will likely trigger a lot of false positives. In an attempt to make hashing deterministic, archetypes, entities and components are sorted by ID before serialization - however I don't think Bevy offers any guarantees on determinism there. 

## Open Questions
* Is `First` early enough in the schedule? How can we best guarantee the hashing runs at the same time every frame?
* How do we handle dynamic types? Something involving `bevy_reflect`?
* There's a couple of manual steps (register components, mark entities for tracking). Can we make this more automatic, and if so, without causing too many false positives?
* Should we take in a "callback" at plugin creation instead of expecting users to 
* Can we stop false positives!
