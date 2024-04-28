use std::future::Future;
use criterion::async_executor::AsyncExecutor;
use criterion::{criterion_group, criterion_main, Criterion};
use nara::runtime::Runtime;

// re-exports for bench_channel.
pub use nara::task;
pub use nara::unsync::mpsc;

// include bench_channel.
mod bench_channel;

struct NaraExecutor;
impl AsyncExecutor for NaraExecutor {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        let runtime = Runtime::new().unwrap();
        runtime.block_on(future)
    }
}

fn bench_channel(c: &mut Criterion) {
    let mut group = c.benchmark_group("nara");
    group.bench_function("channel", |b| {
        b.to_async(NaraExecutor).iter(|| bench_channel::run_bench_channel());
    });
}

criterion_group!(benches, bench_channel);
criterion_main!(benches);
