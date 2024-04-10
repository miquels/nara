use std::time::Duration;

use nara::io::AsyncReadExt;
use nara::runtime::Runtime;
use nara::task::spawn_blocking;
use nara::net::TcpStream;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let runtime = Runtime::new().unwrap();
    let n = runtime.block_on(async {

        let x = spawn_blocking(|| {
            return "foo";
        }).await;
        println!("test1: spawn_blocking returned: {x:?}");

        println!("test2: timer 1 sec");
        nara::time::sleep(Duration::from_millis(1000)).await;
        println!("test2: timer done");

        println!("test3: tcp connection");
        let mut tcp = TcpStream::connect(("smtp.xs4all.nl", 25)).await?;
        println!("test3: connected!");
        let mut buffer: [u8; 256] = [0; 256];
        while let Ok(len) = tcp.read(&mut buffer).await {
            if len == 0 {
                break;
            }
            println!("test3: {:?}", std::str::from_utf8(&buffer[..len]));
            break;
        }
        return Ok::<_, std::io::Error>(3u32);
    })?;
    println!("test4: spawn_blocking return value {n}");

    Ok(())
}


