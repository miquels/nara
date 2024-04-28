use super::mpsc;
use super::task;

pub async fn run_bench_channel() {
    let (tx, mut rx) = mpsc::channel(16);

    let tx_task = task::spawn(async move {
        for i in 0 .. 10_000 {
            let _ = tx.send(i).await.unwrap();
        }
    });

    let rx_task = task::spawn(async move {
        while let Some(_) = rx.recv().await {
            // nothing
        }
    });

    let _ = tx_task.await;
    let _ = rx_task.await;
}

