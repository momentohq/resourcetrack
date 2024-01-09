# resourcetrack

Yet-another resource tracking tool.

# About
A trim, low-dependency tool for counting things. This project does not
use `unsafe` code. It only depends on std.

Safety, clarity, flexibility, and performance are favored in descending order.
Counters are plain structs holding an Arc to an atomic usize. This means there
is a memory cost to the tracking. 0-overhead is not a goal here.

# Details
Resourcetrack supports static categories with your own names:
```rust
use resourcetrack::new_registry;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum MyCategories {
    Miscellaneous,
    Specific,
}

let registry = new_registry::<MyCategories>();
let specific_category_tracker = registry.category(MyCategories::Specific);
```

Resourcetrack does not explicitly synchronize categorized resource counters.
```rust
let _count_sentinel: Count = category_tracker.track(); // non-blocking, on both track and drop
```

Counters are a plain struct, and you can compose them onto your expensive business objects for
automatic count management. If you do this, you can consider using lazy_static for this Registry
to wrap up the counts inside of your constructor function. Ideally you'd cascade the lazy_static
into category Trackers too!
```rust
struct ExpensiveResource {
    payload: String,
    _phantom_count: resourcetrack::tracked::Count,
}
```

When you need to get the counts, for logging or metrics or whatever, just read them.
```rust
let registry = new_registry::<MyCategories>();
{
    let _counter_1 = registry.category(MyCategories::Specific).track();
    assert_eq!(vec![(MyCategories::Specific, 1)], registry.read_counts::<Vec<_>>(), "1 specific instance");
    let _counter_2 = registry.category(MyCategories::Specific).track();
    assert_eq!(vec![(MyCategories::Specific, 2)], registry.read_counts::<Vec<_>>(), "2 specific instances");
}

assert_eq!(vec![(MyCategories::Specific, 0)], registry.read_counts::<Vec<_>>(), "both dropped");
```

You can track sized resources, where their size changes. To stay sane, you should probably limit
yourself to either using track() or track_sized() for a given category. You can mix counts and sizes
within a registry though, no problem!
Complete example:
```rust
use resourcetrack::tracked;

// Set up your statically knowable categories
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum MyCategories {
    ResourceCount,
    ResourceWeight,
}

// Here is an example of a tracked business object. Both size and weight are tracked.
struct TrackedVector {
    internal: Vec<String>,
    _count_sentinel: tracked::Count,
    weight: tracked::Size,
}
impl TrackedVector {
    pub fn push(&mut self, next: String) {
        self.weight.add(next.len());
    }
}

// Static setup - this should be in some shared lazy static scope.
let registry = resourcetrack::new_registry::<MyCategories>();
let resource_counts = registry.category(MyCategories::ResourceCount);
let resource_weights = registry.category(MyCategories::ResourceWeight);

let mut v = TrackedVector { // This should be wrapped into TrackedVector::new() in your application
    internal: Default::default(),
    _count_sentinel: resource_counts.track(),
    weight: resource_weights.track_size(0),
};
v.push("hello".to_string());
//!
let mut counts = registry.read_counts::<Vec<_>>();
counts.sort();
assert_eq!(
    vec![
        (MyCategories::ResourceCount, 1),
        (MyCategories::ResourceWeight, 5),
    ],
    counts,
)
```
