use bit_vec::BitVec;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use slim_datapath::tables::pool::Pool;
use std::mem::MaybeUninit;

fn bench_lookup(c: &mut Criterion) {
    let mut pool = Pool::with_capacity(1024);
    for i in 0..1024 {
        pool.insert(i);
    }

    c.bench_function("pool lookup", |b| {
        b.iter(|| {
            for i in 0..1024 {
                black_box(pool.get(i));
            }
        })
    });
}

fn bench_bitvec_lookup(c: &mut Criterion) {
    let size = 1024;
    let bitvec = BitVec::from_elem(size, true);

    c.bench_function("bitvec lookup", |b| {
        b.iter(|| {
            for i in 0..size {
                black_box(bitvec.get(i));
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

criterion_group!(
    benches,
    bench_lookup,
    bench_bitvec_lookup,
    bench_assume_init_ref,
    bench_insert,
    bench_grow,
    bench_capacity,
);

criterion_main!(benches);
