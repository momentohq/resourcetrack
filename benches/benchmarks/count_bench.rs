use criterion::{criterion_group, Criterion};
use resourcetrack::new_registry;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Categories {
    Miscellaneous,
    // Specific,
}

fn counts(c: &mut Criterion) {
    let mut group = c.benchmark_group("track");

    let registry = new_registry::<Categories>();

    group.bench_with_input("uncached", &registry, |bencher, registry| {
        bencher.iter(|| {
            criterion::black_box(registry.category(Categories::Miscellaneous).track());
        })
    });

    // Cache your categories whenever possible. It takes half as much time to track this way.
    let category_tracker = registry.category(Categories::Miscellaneous);
    group.bench_with_input(
        "cached_category",
        &category_tracker,
        |bencher, category_tracker| {
            bencher.iter(|| {
                criterion::black_box(category_tracker.track());
            })
        },
    );
}

criterion_group!(benches, counts);
