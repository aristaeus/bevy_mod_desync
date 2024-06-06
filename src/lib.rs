use bevy_app::{App, First, Plugin};
use bevy_ecs::{
    component::{Component, ComponentId},
    entity::{Entity, EntityMapper},
    ptr::Ptr,
    system::Resource,
    world::World,
};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

/// Bevy Plugin to detect desyncs
pub struct DesyncPlugin {
    /// Whether to add the update_crc system. Set to false if you want to add this yourself to
    /// control execution
    pub add_system: bool,
    /// Function for sorting entities before hashing. A default implementation which will likely
    /// trigger false positives is provided.
    pub entity_sort: Arc<Box<dyn Fn(&World) -> Vec<Entity> + Send + Sync>>,
}

impl Default for DesyncPlugin {
    fn default() -> Self {
        DesyncPlugin {
            add_system: true,
            entity_sort: Arc::new(Box::new(sort_entities_ids)),
        }
    }
}

impl Plugin for DesyncPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DesyncPluginData {
            entity_sort: self.entity_sort.clone(),
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
pub struct DesyncPluginData {
    serialize_fn_registry: HashMap<ComponentId, unsafe fn(Ptr) -> String>,
    pub entity_sort: Arc<Box<dyn Fn(&World) -> Vec<Entity> + Send + Sync>>,
}

impl Default for DesyncPluginData {
    fn default() -> Self {
        DesyncPluginData {
            serialize_fn_registry: HashMap::default(),
            entity_sort: Arc::new(Box::new(sort_entities_ids)),
        }
    }
}

impl DesyncPluginData {
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
        let mut desync_data = self.world.resource_mut::<DesyncPluginData>();
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

fn get_tracked_components(entity: Entity, world: &World) -> Vec<ComponentId> {
    let entity = world.get_entity(entity).unwrap();
    let archetype = entity.archetype();
    let desync_data = world.resource::<DesyncPluginData>();
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
pub fn sort_entities_ids(world: &World) -> Vec<Entity> {
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

pub trait EnumerateEntities: EntityMapper {
    /// Get all the entities mapped by this mapper
    fn iter_entities(&self) -> Vec<(Entity, Entity)>;
}

/// Sort entities based on an entity map
///
/// Usage:
/// ```rust,ignore
/// app.add_plugins(
/// DesyncPlugin {
///     entity_sort: Arc::new(Box::new(|w| sort_from_entity_map::<MyEntityMapperType>(w, true))),
///     ..Default::default()
/// })
/// ```
///
/// EntityMapper requires clone because this function requires read only access to the world
pub fn sort_from_entity_map<Mapper: EnumerateEntities + Resource + Clone>(
    world: &World,
    from_self: bool,
) -> Vec<Entity> {
    let mut entity_map = world.get_resource::<Mapper>().unwrap().clone();
    let entities = world
        .iter_entities()
        .filter(|entity| entity.contains::<TrackDesync>())
        .map(|e| e.id());
    if from_self {
        let mut entities = entities.collect::<Vec<_>>();
        entities.sort_by(|a, b| a.cmp(&b));
        entities
    } else {
        // invert entity mapper
        let mut entities = entity_map.iter_entities();
        entities.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        entities
            .iter()
            .map(|e| entity_map.map_entity(e.0))
            .collect()
    }
}

pub fn calculate_crc(world: &World) -> u16 {
    let mut crc_input = String::new();
    let desync_data = world.resource::<DesyncPluginData>();
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
    use bevy_ecs::entity::EntityHashMap;

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

    #[derive(Clone, Default, Resource)]
    struct EntityMap {
        entity_map: EntityHashMap<Entity>,
    }

    impl EntityMapper for EntityMap {
        fn map_entity(&mut self, entity: Entity) -> Entity {
            *self.entity_map.get(&entity).unwrap()
        }
    }

    impl EnumerateEntities for EntityMap {
        fn iter_entities(&self) -> Vec<(Entity, Entity)> {
            self.entity_map.iter().map(|(a, b)| (*a, *b)).collect()
        }
    }

    #[test]
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
        let mut entity_map = EntityHashMap::default();
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
}
