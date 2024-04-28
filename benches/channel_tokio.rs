use std::future::Future;
use criterion::async_executor::AsyncExecutor;
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Builder;

// re-exports for bench_channel.
pub use tokio::task;
pub use tokio::sync::mpsc;

// include bench_channel.
mod bench_channel;

struct TokioExecutor;
impl AsyncExecutor for TokioExecutor {
    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(future)
    }
}

fn bench_channel(c: &mut Criterion) {
    let mut group = c.benchmark_group("tokio");
    group.bench_function("channel", |b| {
        b.to_async(TokioExecutor).iter(|| bench_channel::run_bench_channel());
    });
}

criterion_group!(benches, bench_channel);
criterion_main!(benches);
