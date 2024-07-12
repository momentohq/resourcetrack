//! Yet-another resource tracking tool.
//!
//! # About
//! A trim, low-dependency tool for counting things. This project does not
//! use `unsafe` code. It only depends on std.
//!
//! Safety, clarity, flexibility, and performance are favored in descending order.
//! Counters are plain structs holding an Arc to an atomic usize. This means there
//! is a memory cost to the tracking. 0-overhead is not a goal here.
//!
//! # Details
//! Resourcetrack supports static categories with your own names:
//! ```rust
//! use resourcetrack::new_registry;
//!
//! #[derive(Clone, Debug, PartialEq, Eq, Hash)]
//! enum MyCategories {
//!     Miscellaneous,
//!     Specific,
//! }
//!
//! let registry = new_registry::<MyCategories>();
//! let specific_category_tracker = registry.category(MyCategories::Specific);
//! ```
//!
//! Resourcetrack does not explicitly synchronize categorized resource counters.
//! ```rust
//! # use resourcetrack::new_registry;
//! #
//! # #[derive(Clone, Debug, PartialEq, Eq, Hash)]
//! # enum MyCategories {
//! #     Miscellaneous,
//! #     Specific,
//! # }
//! #
//! # let registry = new_registry::<MyCategories>();
//! # let category_tracker = registry.category(MyCategories::Specific);
//! use resourcetrack::tracked::Count;
//!
//! let _count_sentinel: Count = category_tracker.track(); // non-blocking, on both track and drop
//! ```
//!
//! Counters are a plain struct, and you can compose them onto your expensive business objects for
//! automatic count management. If you do this, you can consider using lazy_static for this Registry
//! to wrap up the counts inside of your constructor function. Ideally you'd cascade the lazy_static
//! into category Trackers too!
//! ```rust
//! struct ExpensiveResource {
//!     payload: String,
//!     _phantom_count: resourcetrack::tracked::Count,
//! }
//! ```
//!
//! When you need to get the counts, for logging or metrics or whatever, just read them.
//! ```rust
//! # use resourcetrack::new_registry;
//! #
//! # #[derive(Clone, Debug, PartialEq, Eq, Hash)]
//! # enum MyCategories {
//! #     Miscellaneous,
//! #     Specific,
//! # }
//! #
//! let registry = new_registry::<MyCategories>();
//! {
//!     let _counter_1 = registry.category(MyCategories::Specific).track();
//!     assert_eq!(vec![(MyCategories::Specific, 1)], registry.read_counts::<Vec<_>>(), "1 specific instance");
//!     let _counter_2 = registry.category(MyCategories::Specific).track();
//!     assert_eq!(vec![(MyCategories::Specific, 2)], registry.read_counts::<Vec<_>>(), "2 specific instances");
//! }
//!
//! assert_eq!(vec![(MyCategories::Specific, 0)], registry.read_counts::<Vec<_>>(), "both dropped");
//! ```
//!
//! You can track sized resources, where their size changes. To stay sane, you should probably limit
//! yourself to either using track() or track_sized() for a given category. You can mix counts and sizes
//! within a registry though, no problem!
//! Complete example:
//! ```rust
//! use resourcetrack::tracked;
//!
//! // Set up your statically knowable categories
//! #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
//! enum MyCategories {
//!     ResourceCount,
//!     ResourceWeight,
//! }
//!
//! // Here is an example of a tracked business object
//! struct TrackedVector {
//!     internal: Vec<String>,
//!     _count_sentinel: tracked::Count,
//!     weight: tracked::Size,
//! }
//! impl TrackedVector {
//!     pub fn push(&mut self, next: String) {
//!         self.weight.add(next.len());
//!     }
//! }
//!
//! // Static setup - this should be in some shared lazy static scope.
//! let registry = resourcetrack::new_registry::<MyCategories>();
//! let resource_counts = registry.category(MyCategories::ResourceCount);
//! let resource_weights = registry.category(MyCategories::ResourceWeight);
//!
//! let mut v = TrackedVector { // This should be wrapped into TrackedVector::new() in your application
//!     internal: Default::default(),
//!     _count_sentinel: resource_counts.track(),
//!     weight: resource_weights.track_size(0),
//! };
//! v.push("hello".to_string());
//!
//! let mut counts = registry.read_counts::<Vec<_>>();
//! counts.sort();
//! assert_eq!(
//!     vec![
//!         (MyCategories::ResourceCount, 1),
//!         (MyCategories::ResourceWeight, 5),
//!     ],
//!     counts,
//! )
//! ```

use std::{
    borrow::Borrow,
    collections::HashMap,
    fmt::Debug,
    sync::{atomic::AtomicUsize, Arc, Mutex},
};

pub struct Registry<Id> {
    categories: Mutex<HashMap<Id, Category>>,
}

impl<Id> Debug for Registry<Id>
where
    Id: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("categories", &self.categories.lock().expect("local mutex"))
            .finish()
    }
}

/// Create a new registry keyed by its ID type.
///
/// Enums are the recommended ID types, and `&'static str` is pretty good too.
///
/// If you need to track dynamic ids, consider using Arc<String> or something like that
/// so that Clone is not too costly.
pub fn new_registry<Id>() -> Registry<Id>
where
    Id: Debug + Eq + std::hash::Hash + Clone,
{
    Registry {
        categories: Default::default(),
    }
}

impl<Id> Registry<Id>
where
    Id: Debug + Eq + std::hash::Hash + Clone,
{
    /// You should cache the tracker. Getting a reference requires a mutex interaction.
    /// It's fine to do it occasionally, or in non-latency-sensitive paths, but this is
    /// not an optimized path. Tracker and Count are quick.
    pub fn category<Name>(&self, name: Name) -> Tracker
    where
        Name: Into<Id> + std::hash::Hash + std::cmp::Eq,
        Id: Borrow<Name>,
    {
        let mut categories = self.categories.lock().expect("local mutex");
        let count = match categories.get(&name) {
            Some(existing) => existing.total.clone(),
            None => {
                let count = Arc::new(AtomicUsize::new(0));
                categories.insert(
                    name.into(),
                    Category {
                        total: count.clone(),
                    },
                );
                count
            }
        };
        Tracker { count }
    }

    /// This is appropriate for infrequent access - e.g., for polling metrics every few seconds.
    /// It walks the categories and loads counts. Consider reading into a vector instead of a map.
    ///
    /// This function contends with category(). Try to get your category trackers up front and use
    /// this only in a background job.
    pub fn read_counts<AsCollection>(&self) -> AsCollection
    where
        AsCollection: FromIterator<(Id, usize)>,
    {
        let categories = self.categories.lock().expect("local mutex");
        categories
            .iter()
            .map(|(id, category)| (id.clone(), category.total()))
            .collect()
    }
}

#[derive(Clone)]
struct Category {
    total: Arc<AtomicUsize>,
}

impl Debug for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Category")
            .field("total", &self.total())
            .finish()
    }
}

impl Category {
    pub fn total(&self) -> usize {
        self.total.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct Tracker {
    count: Arc<AtomicUsize>,
}

impl Debug for Tracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.count.load(std::sync::atomic::Ordering::Relaxed)
        )
    }
}

impl Tracker {
    /// Hold 1 count against the category until the returned tracked::Count guard is dropped.
    pub fn track(&self) -> tracked::Count {
        self.count
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        tracked::Count {
            total: self.count.clone(),
        }
    }

    /// Hold a count against the category until the returned tracked::Size guard is dropped.
    /// A Size guard can be mutated. For example, you might use this with a "buffers"
    /// resource category: When you change the buffer size you can also update the tracked::Size
    /// for better visibility into where your memory is spent.
    pub fn track_size(&self, initial: usize) -> tracked::Size {
        self.count
            .fetch_add(initial, std::sync::atomic::Ordering::Release);
        tracked::Size {
            total: self.count.clone(),
            local: initial,
        }
    }
}

pub mod tracked {
    use std::{
        fmt::Debug,
        sync::{atomic::AtomicUsize, Arc},
    };

    /// Fixed handle for a resource that is only counted by its existence.
    pub struct Count {
        pub(crate) total: Arc<AtomicUsize>,
    }

    impl Debug for Count {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "Count: {}",
                self.total.load(std::sync::atomic::Ordering::Relaxed)
            )
        }
    }

    impl Drop for Count {
        fn drop(&mut self) {
            self.total
                .fetch_sub(1, std::sync::atomic::Ordering::Release);
        }
    }

    /// Mutable handle for a resource of changing size.
    pub struct Size {
        pub(crate) total: Arc<AtomicUsize>,
        pub(crate) local: usize,
    }

    impl Debug for Size {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Size")
                .field(
                    "total",
                    &self.total.load(std::sync::atomic::Ordering::Relaxed),
                )
                .field("local", &self.local)
                .finish()
        }
    }

    impl Size {
        /// change the tracked count for this resource
        pub fn set(&mut self, new_size: usize) {
            let difference = new_size.abs_diff(self.local);
            if new_size < self.local {
                self.total
                    .fetch_sub(difference, std::sync::atomic::Ordering::Release);
            } else {
                self.total
                    .fetch_add(difference, std::sync::atomic::Ordering::Release);
            }
            self.local = new_size;
        }

        /// change the tracked count for this resource
        pub fn add(&mut self, amount: usize) {
            self.total
                .fetch_add(amount, std::sync::atomic::Ordering::Release);
            self.local += amount;
        }

        /// change the tracked count for this resource
        pub fn subtract(&mut self, amount: usize) {
            self.total.fetch_sub(
                std::cmp::min(amount, self.local),
                std::sync::atomic::Ordering::Release,
            );
            self.local = self.local.saturating_sub(amount);
        }
    }

    impl Drop for Size {
        fn drop(&mut self) {
            self.total
                .fetch_sub(self.local, std::sync::atomic::Ordering::Release);
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::new_registry;

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    enum Categories {
        Miscellaneous,
        SpecificOne,
    }

    type CountsVec = Vec<(Categories, usize)>;

    #[test]
    fn count_follows_counters() {
        let registry = new_registry::<Categories>();

        assert_eq!(CountsVec::new(), registry.read_counts::<Vec<_>>());

        let miscellaneous_tracker = registry.category(Categories::Miscellaneous);
        assert_eq!(
            vec![(Categories::Miscellaneous, 0)],
            registry.read_counts::<Vec<_>>()
        );

        {
            let _counter = miscellaneous_tracker.track();
            assert_eq!(
                vec![(Categories::Miscellaneous, 1)],
                registry.read_counts::<Vec<_>>()
            );
        }
        assert_eq!(
            vec![(Categories::Miscellaneous, 0)],
            registry.read_counts::<Vec<_>>()
        );
    }

    #[test]
    fn count_does_not_reset_on_read() {
        let registry = new_registry::<Categories>();
        let category_tracker = registry.category(Categories::SpecificOne);

        let _counter = category_tracker.track();
        assert_eq!(
            vec![(Categories::SpecificOne, 1)],
            registry.read_counts::<Vec<_>>()
        );
        assert_eq!(
            vec![(Categories::SpecificOne, 1)],
            registry.read_counts::<Vec<_>>()
        );
    }

    #[test]
    fn static_string_registry() {
        let registry = new_registry::<&'static str>();
        let category_tracker = registry.category("plain string category");

        let _counter = category_tracker.track();
        assert_eq!(
            vec![("plain string category", 1)],
            registry.read_counts::<Vec<_>>()
        );
    }

    #[test]
    fn string_registry() {
        // PLEASE use a smart pointer for Strings and other expensive-clone catrgory types!
        // Reading counts from the Registry relies on clone(), and String clone() is expensive!

        // Sometimes you may need dynamic categories - for instance, to record usage per user.
        // In those cases, you can consider splitting your registries between a faster enum for
        // your statically known categories and your dynamically resolved categories.
        // You should still try to cache your category_trackers as best you can. Each lookup
        // of a category in the registry is mutually synchronized.
        let registry = new_registry::<Arc<String>>();
        let category_tracker = registry.category(Arc::new("dynamic".into()));

        let _counter = category_tracker.track();
        assert_eq!(
            vec![(Arc::new("dynamic".into()), 1)],
            registry.read_counts::<Vec<_>>()
        );
    }

    #[test]
    fn add() {
        let registry = new_registry::<Categories>();
        let category_tracker = registry.category(Categories::SpecificOne);

        {
            let mut size = category_tracker.track_size(4);
            assert_eq!(
                vec![(Categories::SpecificOne, 4)],
                registry.read_counts::<Vec<_>>()
            );

            size.add(3);

            assert_eq!(
                vec![(Categories::SpecificOne, 7)],
                registry.read_counts::<Vec<_>>()
            );
        }
        assert_eq!(
            vec![(Categories::SpecificOne, 0)],
            registry.read_counts::<Vec<_>>()
        );
    }

    #[test]
    fn subtract() {
        let registry = new_registry::<Categories>();
        let category_tracker = registry.category(Categories::SpecificOne);

        {
            let mut size = category_tracker.track_size(4);

            size.subtract(2);

            assert_eq!(
                vec![(Categories::SpecificOne, 2)],
                registry.read_counts::<Vec<_>>()
            );
        }
        assert_eq!(
            vec![(Categories::SpecificOne, 0)],
            registry.read_counts::<Vec<_>>()
        );
    }

    #[test]
    fn subtract_does_not_wrap() {
        let registry = new_registry::<Categories>();
        let category_tracker = registry.category(Categories::SpecificOne);

        {
            let mut size = category_tracker.track_size(4);

            size.subtract(5); // probably a bug in your code - let's not make it worse.

            assert_eq!(
                vec![(Categories::SpecificOne, 0)],
                registry.read_counts::<Vec<_>>()
            );
        }
        assert_eq!(
            vec![(Categories::SpecificOne, 0)],
            registry.read_counts::<Vec<_>>()
        );
    }

    #[test]
    fn size_set() {
        let registry = new_registry::<Categories>();
        let category_tracker = registry.category(Categories::SpecificOne);

        {
            let mut size = category_tracker.track_size(4);

            size.set(5);
            assert_eq!(
                vec![(Categories::SpecificOne, 5)],
                registry.read_counts::<Vec<_>>()
            );

            size.set(1);
            assert_eq!(
                vec![(Categories::SpecificOne, 1)],
                registry.read_counts::<Vec<_>>()
            );
        }
        assert_eq!(
            vec![(Categories::SpecificOne, 0)],
            registry.read_counts::<Vec<_>>()
        );
    }
}
