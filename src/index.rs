use crate::unique_multimap::UniqueMultiMap;
use bevy::ecs::archetype::Archetype;
use bevy::ecs::change_detection::Ref;
use bevy::ecs::component::Tick;
use bevy::ecs::system::{ReadOnlySystemParam, SystemMeta, SystemParam};
use bevy::prelude::*;
use bevy::utils::HashSet;
use std::hash::Hash;

pub trait IndexInfo {
    type Component: Component;
    type Value: Send + Sync + Hash + Eq + Clone;

    fn value(c: &Self::Component) -> Self::Value;
}

#[derive(Resource)]
pub struct IndexStorage<I: IndexInfo> {
    map: UniqueMultiMap<I::Value, Entity>,
    last_refresh_tick: u32,
}
impl<I: IndexInfo> Default for IndexStorage<I> {
    fn default() -> Self {
        IndexStorage {
            map: Default::default(),
            last_refresh_tick: 0,
        }
    }
}

type ComponetsQuery<'w, 's, T> = Query<'w, 's, (Entity, Ref<'static, <T as IndexInfo>::Component>)>;

pub struct Index<'w, 's, T: IndexInfo + 'static> {
    storage: ResMut<'w, IndexStorage<T>>,
    components: ComponetsQuery<'w, 's, T>,
    current_tick: u32,
}

impl<'w, 's, T: IndexInfo> Index<'w, 's, T> {
    pub fn lookup(&mut self, val: &T::Value) -> HashSet<Entity> {
        self.refresh();
        self.storage.map.get(val)
    }

    pub fn refresh(&mut self) {
        if self.storage.last_refresh_tick >= self.current_tick {
            return; // Already updated in this system.
        }

        for (entity, component) in &self.components {
            // Subtract 1 so that changes from the system where the index was updated are seen.
            // The `changed` implementation assumes we don't care about those changes since
            // "this" system is the one that made the change, but for indexing, we do care.
            if Tick::new(self.storage.last_refresh_tick.wrapping_sub(1))
                .is_older_than(component.last_changed(), self.current_tick)
            {
                println!("update val for {:?}", entity);
                self.storage.map.insert(&T::value(&component), &entity);
            }
        }
        self.storage.last_refresh_tick = self.current_tick;
    }
}

pub struct IndexFetchState<'w, 's, T: IndexInfo + 'static> {
    storage_state: <ResMut<'w, IndexStorage<T>> as SystemParam>::State,
    changed_components_state: <ComponetsQuery<'w, 's, T> as SystemParam>::State,
}
unsafe impl<'w, 's, T: IndexInfo + 'static> SystemParam for Index<'w, 's, T> {
    type State = IndexFetchState<'static, 'static, T>;
    type Item<'_w, '_s> = Index<'_w, '_s, T>;
    fn init_state(world: &mut World, system_meta: &mut SystemMeta) -> Self::State {
        world.init_resource::<IndexStorage<T>>();
        IndexFetchState {
            storage_state: <ResMut<'w, IndexStorage<T>> as SystemParam>::init_state(
                world,
                system_meta,
            ),
            changed_components_state: <ComponetsQuery<'w, 's, T> as SystemParam>::init_state(
                world,
                system_meta,
            ),
        }
    }
    fn new_archetype(state: &mut Self::State, archetype: &Archetype, system_meta: &mut SystemMeta) {
        <ResMut<'w, IndexStorage<T>> as SystemParam>::new_archetype(
            &mut state.storage_state,
            archetype,
            system_meta,
        );
        <ComponetsQuery<'w, 's, T> as SystemParam>::new_archetype(
            &mut state.changed_components_state,
            archetype,
            system_meta,
        );
    }
    fn apply(state: &mut Self::State, system_meta: &SystemMeta, world: &mut World) {
        <ResMut<'w, IndexStorage<T>> as SystemParam>::apply(
            &mut state.storage_state,
            system_meta,
            world,
        );
        <ComponetsQuery<'w, 's, T> as SystemParam>::apply(
            &mut state.changed_components_state,
            system_meta,
            world,
        );
    }
    unsafe fn get_param<'w2, 's2>(
        state: &'s2 mut Self::State,
        system_meta: &SystemMeta,
        world: &'w2 World,
        change_tick: u32,
    ) -> Self::Item<'w2, 's2> {
        Index {
            storage: <ResMut<'w, IndexStorage<T>>>::get_param(
                &mut state.storage_state,
                system_meta,
                world,
                change_tick,
            ),
            components: <ComponetsQuery<'w, 's, T> as SystemParam>::get_param(
                &mut state.changed_components_state,
                system_meta,
                world,
                change_tick,
            ),
            current_tick: change_tick,
        }
    }
}
unsafe impl<'w, 's, T: IndexInfo + 'static> ReadOnlySystemParam for Index<'w, 's, T>
where
    ResMut<'w, IndexStorage<T>>: ReadOnlySystemParam,
    ComponetsQuery<'w, 's, T>: ReadOnlySystemParam,
{
}

mod test {
    use crate::prelude::*;
    use bevy::prelude::*;

    #[derive(Component, Clone, Eq, Hash, PartialEq, Debug)]
    struct Number(usize);

    //todo: maybe make this a derive macro
    impl IndexInfo for Number {
        type Component = Self;
        type Value = Self;

        fn value(c: &Self::Component) -> Self::Value {
            c.clone()
        }
    }

    fn add_some_numbers(mut commands: Commands) {
        commands.spawn(Number(10));
        commands.spawn(Number(10));
        commands.spawn(Number(20));
        commands.spawn(Number(30));
    }

    fn checker(number: usize, amount: usize) -> impl Fn(Index<Number>) {
        move |mut idx: Index<Number>| {
            let set = idx.lookup(&Number(number));
            assert_eq!(
                set.len(),
                amount,
                "Index returned {} matches for {}, expectd {}.",
                set.len(),
                number,
                amount,
            );
            println!("DONE CHEKING")
        }
    }

    fn adder_all(n: usize) -> impl Fn(Query<&mut Number>) {
        move |mut nums: Query<&mut Number>| {
            for mut num in &mut nums {
                num.0 += n;
            }
        }
    }

    fn adder_some(
        n: usize,
        condition: usize,
    ) -> impl Fn(ParamSet<(Query<&mut Number>, Index<Number>)>) {
        move |mut nums_and_index: ParamSet<(Query<&mut Number>, Index<Number>)>| {
            for entity in nums_and_index.p1().lookup(&Number(condition)).into_iter() {
                let mut nums = nums_and_index.p0();
                let mut nref: Mut<Number> = nums.get_mut(entity).unwrap();
                nref.0 += n;
            }
        }
    }

    #[test]
    fn test_index_lookup() {
        App::new()
            .add_startup_system(add_some_numbers)
            .add_system(checker(10, 2))
            .add_system(checker(20, 1))
            .add_system(checker(30, 1))
            .add_system(checker(40, 0))
            .run();
    }

    #[test]
    fn test_changing_values() {
        App::new()
            .add_startup_system(add_some_numbers)
            .add_system_to_stage(CoreStage::PreUpdate, checker(10, 2))
            .add_system_to_stage(CoreStage::PreUpdate, checker(20, 1))
            .add_system_to_stage(CoreStage::PreUpdate, checker(30, 1))
            .add_system(adder_all(5))
            .add_system_to_stage(CoreStage::PostUpdate, checker(10, 0))
            .add_system_to_stage(CoreStage::PostUpdate, checker(20, 0))
            .add_system_to_stage(CoreStage::PostUpdate, checker(30, 0))
            .add_system_to_stage(CoreStage::PostUpdate, checker(15, 2))
            .add_system_to_stage(CoreStage::PostUpdate, checker(25, 1))
            .add_system_to_stage(CoreStage::PostUpdate, checker(35, 1))
            .run();
    }

    #[test]
    fn test_changing_with_index() {
        App::new()
            .add_startup_system(add_some_numbers)
            .add_system_to_stage(CoreStage::PreUpdate, checker(10, 2))
            .add_system_to_stage(CoreStage::PreUpdate, checker(20, 1))
            .add_system(adder_some(10, 10))
            .add_system_to_stage(CoreStage::PostUpdate, checker(10, 0))
            .add_system_to_stage(CoreStage::PostUpdate, checker(20, 3))
            .run();
    }
}
