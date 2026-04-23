use criterion::{Criterion, black_box, criterion_group, criterion_main};
use slim_datapath::tables::pool::Pool;
use std::mem::MaybeUninit;

fn bench_lookup(c: &mut Criterion) {
    let mut pool = Pool::with_capacity(1024);
    let ids: Vec<u64> = (0..1024i32).map(|i| pool.insert(i)).collect();

    c.bench_function("pool lookup", |b| {
        b.iter(|| {
            for &id in &ids {
                black_box(pool.get(id));
            }
        })
    });
}

fn bench_assume_init_ref(c: &mut Criterion) {
    let size = 1024;
    let pool: Vec<MaybeUninit<i32>> = {
        let mut v = Vec::with_capacity(size);
        v.resize_with(size, || MaybeUninit::new(42));
        v
    };

    c.bench_function("assume_init_ref only", |b| {
        b.iter(|| {
            for item in pool.iter() {
                let _ = black_box(unsafe { item.assume_init_ref() });
            }
        })
    });
}

fn bench_insert(c: &mut Criterion) {
    c.bench_function("pool insert", |b| {
        b.iter(|| {
            let mut pool = Pool::with_capacity(1024);
            for i in 0..1024 {
                pool.insert(i);
            }
        })
    });
}

fn bench_grow(c: &mut Criterion) {
    c.bench_function("pool grow", |b| {
        b.iter(|| {
            let mut pool = Pool::with_capacity(16);
            for i in 0..1024 {
                pool.insert(i);
            }
        })
    });
}

fn bench_capacity(c: &mut Criterion) {
    c.bench_function("pool capacity", |b| {
        b.iter(|| {
            let pool: Pool<i32> = Pool::with_capacity(1024);
            let _ = pool.capacity();
        })
    });
}

fn bench_remove(c: &mut Criterion) {
    c.bench_function("pool remove", |b| {
        b.iter(|| {
            let mut pool = Pool::with_capacity(1024);
            let ids: Vec<u64> = (0..1024).map(|i| pool.insert(i)).collect();
            for id in &ids {
                black_box(pool.remove(*id));
            }
        })
    });
}

fn bench_remove_insert_cycle(c: &mut Criterion) {
    // Measures the cost of alternating remove+insert, exercising slot reuse.
    c.bench_function("pool remove+insert cycle", |b| {
        b.iter(|| {
            let mut pool = Pool::with_capacity(1024);
            let ids: Vec<u64> = (0..1024).map(|i| pool.insert(i)).collect();
            for (i, &id) in ids.iter().enumerate() {
                pool.remove(id);
                black_box(pool.insert(i as i32));
            }
        })
    });
}

fn bench_next_id(c: &mut Criterion) {
    // Representative pool size: a handful of outbound connections per name.
    let mut pool = Pool::with_capacity(8);
    for i in 0..8i32 {
        pool.insert(i);
    }

    c.bench_function("pool next_id", |b| {
        b.iter(|| black_box(pool.next_id()))
    });
}

fn bench_next_val(c: &mut Criterion) {
    let mut pool = Pool::with_capacity(8);
    for i in 0..8i32 {
        pool.insert(i);
    }

    c.bench_function("pool next_val", |b| {
        b.iter(|| black_box(pool.next_val()))
    });
}

criterion_group!(
    benches,
    bench_lookup,
    bench_assume_init_ref,
    bench_insert,
    bench_grow,
    bench_capacity,
    bench_remove,
    bench_remove_insert_cycle,
    bench_next_id,
    bench_next_val,
);

criterion_main!(benches);
