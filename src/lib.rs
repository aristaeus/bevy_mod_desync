use bevy_app::{App, First, Plugin};
use bevy_ecs::{
    component::{Component, ComponentId},
    entity::Entity,
    ptr::Ptr,
    system::Resource,
    world::World,
};
use serde::Serialize;
use std::collections::HashMap;

/// Bevy Plugin to detect desyncs
pub struct DesyncPlugin {
    /// Whether to add the update_crc system. Set to false if you want to add this yourself to
    /// control execution
    pub add_system: bool,
    /// Function for sorting entities before hashing. A default implementation which will likely
    /// trigger false positives is provided.
    pub entity_sort: fn(&World) -> Vec<Entity>,
}

impl Default for DesyncPlugin {
    fn default() -> Self {
        DesyncPlugin {
            add_system: true,
            entity_sort: sort_entities_ids,
        }
    }
}

impl Plugin for DesyncPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DesyncDataRegistries {
            entity_sort: self.entity_sort,
            ..Default::default()
        })
        .init_resource::<Crc>();
        app.world.init_component::<TrackDesync>();

        if self.add_system {
            app.add_systems(First, update_crc);
        }
    }
}

/// CRC Resource - contains the hash of the ECS world at the start of the tick
#[derive(Debug, Default, PartialEq, Resource)]
pub struct Crc(pub u16);

#[derive(Resource)]
struct DesyncDataRegistries {
    serialize_fn_registry: HashMap<ComponentId, unsafe fn(Ptr) -> String>,
    entity_sort: fn(&World) -> Vec<Entity>,
}

impl Default for DesyncDataRegistries {
    fn default() -> Self {
        DesyncDataRegistries {
            serialize_fn_registry: HashMap::default(),
            entity_sort: sort_entities_ids,
        }
    }
}

impl DesyncDataRegistries {
    fn serialize(&self, ptr: Ptr, id: &ComponentId) -> String {
        unsafe {
            // SAFETY: components match
            self.serialize_fn_registry[id](ptr)
        }
    }
}

/// Component to mark an entity for desync tracking
#[derive(Component)]
pub struct TrackDesync;

// to track an entity we need:
// * component marked with app.track_desync()
// * entity marked with TrackDesync
// * plugin added
// * component impl serialize
// OPEN QUESTIONS
// * is tracking opt in or opt out?
// * is component registering required?

pub trait AppDesyncExt {
    fn track_desync<T: Component + Serialize>(&mut self);
}

impl AppDesyncExt for App {
    fn track_desync<T: Component + Serialize>(&mut self) {
        let component_id = self.world.init_component::<T>();
        let mut desync_data = self.world.resource_mut::<DesyncDataRegistries>();
        desync_data
            .serialize_fn_registry
            .insert(component_id, untyped_serialize::<T>);
    }
}

/// SAFETY: Ptr must be of type T
unsafe fn untyped_serialize<T: Component + Serialize>(ptr: Ptr) -> String {
    let se = ptr.deref::<T>();
    serde_json::to_string(se).unwrap()
}

pub fn get_tracked_components(entity: Entity, world: &World) -> Vec<ComponentId> {
    let entity = world.get_entity(entity).unwrap();
    let archetype = entity.archetype();
    let desync_data = world.resource::<DesyncDataRegistries>();
    let mut components = archetype
        .components()
        .filter(|c| desync_data.serialize_fn_registry.contains_key(c))
        .collect::<Vec<_>>();
    // TODO: component IDs aren't stable, think of a better way to sort
    components.sort();
    components
}

/// This method of calculating the CRC sorts archetypes, entities and components by their IDs. This
/// may lead to false positives if the two worlds have different orders for those IDs.
fn sort_entities_ids(world: &World) -> Vec<Entity> {
    let track_desync_component_id = world.component_id::<TrackDesync>().unwrap();
    let mut archetypes = world
        .archetypes()
        .iter()
        // archetypes with the track_desync component
        .filter(|a| a.contains(track_desync_component_id))
        .collect::<Vec<_>>();
    // TODO: archetype IDs aren't stable, think of a better way to sort
    archetypes.sort_by(|a, b| a.id().cmp(&b.id()));

    archetypes
        .iter()
        .flat_map(|archetype| {
            let mut entities = archetype.entities().iter().collect::<Vec<_>>();
            // TODO: entity IDs aren't stable, think of a better way to sort
            entities.sort_by(|a, b| a.id().cmp(&b.id()));
            entities.iter().map(|e| e.id()).collect::<Vec<_>>()
        })
        .collect()
}

pub fn calculate_crc(world: &World) -> u16 {
    let mut crc_input = String::new();
    let desync_data = world.resource::<DesyncDataRegistries>();
    let entities = (desync_data.entity_sort)(world);
    for entity in entities.iter() {
        let components = get_tracked_components(*entity, world);
        // check has tracking
        if !world.get_entity(*entity).unwrap().contains::<TrackDesync>() {
            continue;
        }
        for c in components.iter() {
            let ptr = world.get_by_id(*entity, *c).unwrap();
            crc_input.push_str(&desync_data.serialize(ptr, c));
        }
    }

    let crc_algo = crc::Crc::<u16>::new(&crc::CRC_16_IBM_SDLC);
    crc_algo.checksum(crc_input.as_bytes())
}

pub fn update_crc(world: &mut World) {
    let crc = calculate_crc(world);
    let mut crc_res = world.resource_mut::<Crc>();
    *crc_res = Crc(crc);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Component, Serialize)]
    struct Foo(u64);

    fn build_app() -> App {
        let mut app = App::new();
        app.add_plugins(DesyncPlugin::default())
            .track_desync::<Foo>();
        app
    }

    #[test]
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

    #[test]
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

    #[test]
    fn entity_spawn_order() {
        let mut app_1 = build_app();
        let mut app_2 = build_app();
        app_1.world.spawn((Foo(0), TrackDesync));
        app_1.world.spawn((Foo(1), TrackDesync));
        app_2.world.spawn((Foo(1), TrackDesync));
        app_2.world.spawn((Foo(0), TrackDesync));

        // calculate crc
        app_1.update();
        app_2.update();

        assert_eq!(app_1.world.resource::<Crc>(), app_2.world.resource::<Crc>());
    }
}
