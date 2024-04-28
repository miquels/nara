use std::thread::sleep as std_sleep;
use std::time::Duration;

use nara::io::AsyncReadExt;
use nara::runtime::Runtime;
use nara::task;
use nara::time::sleep;
use nara::net::TcpStream;
use nara::unsync::mpsc::channel;

async fn test_channel() {
    let (tx1, rx) = channel(4);
    let tx2 = tx1.clone();

    let ping1 = task::spawn(async move {
        println!("test_channel: start sender 1");
        for i in 1 ..=10 {
            tx1.send(i).await.unwrap();
            sleep(Duration::from_millis(100)).await;
        }
    });

    let ping2 = task::spawn(async move {
        println!("test_channel: start sender 2");
        sleep(Duration::from_millis(50)).await;
        for i in 11 ..=20 {
            tx2.send(i).await.unwrap();
            sleep(Duration::from_millis(100)).await;
        }
    });

    let pong = task::spawn(async move {
        sleep(Duration::from_millis(500)).await;
        println!("test_channel: start receiver");
        while let Some(num) = rx.recv().await {
            println!("test_channel: recv {num}");
        }
    });

    let _ = ping1.await;
    let _ = ping2.await;
    let _ = pong.await;
    println!("test_channel: done");
}

async fn test_spawn_blocking() {
    async fn bl_spawn(id: u64) -> String {
        task::spawn_blocking(move || {
            std_sleep(Duration::from_millis(250));
            format!("foo {}", id)
        }).await.unwrap()
    }
    let b1 = bl_spawn(1);
    let b2 = bl_spawn(2);
    let b3 = bl_spawn(3);
    let b4 = bl_spawn(4);
    println!("test_spawn_blocking: returned: {:?}", futures::join!(b1, b2, b3, b4));
}

async fn test_sleep() {
    println!("test_sleep: timer 1 sec");
    sleep(Duration::from_millis(1000)).await;
    println!("test_sleep: timer done");
}

async fn test_tcp() -> std::io::Result<()> {
        println!("test_tcp: open tcp connection");
        let mut tcp = TcpStream::connect(("smtp.bit.nl", 25)).await?;
        println!("test_tcp: connected!");
        let mut buffer: [u8; 256] = [0; 256];
        while let Ok(len) = tcp.read(&mut buffer).await {
            if len == 0 {
                break;
            }
            println!("test_tcp: {:?}", std::str::from_utf8(&buffer[..len]));
            break;
        }
        Ok(())
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let runtime = Runtime::new().unwrap();
    let n = runtime.block_on(async {

        test_channel().await;
        test_sleep().await;
        test_spawn_blocking().await;
        test_sleep().await;
        let _ = test_tcp().await.map_err(|e| println!("test_tcp: error: {}", e));
        return Ok::<_, std::io::Error>(3u32);
    });

    println!("final: block_on return value {:?}", n);

    Ok(())
}

