use std::thread::sleep;
use std::time::Duration;

use kyoto::io::AsyncReadExt;
use kyoto::runtime::Runtime;
use kyoto::task::spawn_blocking;
use kyoto::net::TcpStream;

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let runtime = Runtime::new().unwrap();
    let n = runtime.block_on(async {

        let x = spawn_blocking(|| {
            sleep(Duration::from_millis(1000));
            return "foo";
        }).await;
        println!("returned: {x:?}");

        let mut tcp = TcpStream::connect(("smtp.xs4all.nl", 25)).await?;
        println!("connected!");
        let mut buffer: [u8; 256] = [0; 256];
        while let Ok(len) = tcp.read(&mut buffer).await {
            if len == 0 {
                break;
            }
            println!("{:?}", std::str::from_utf8(&buffer[..len]));
            break;
        }
        return Ok::<_, std::io::Error>(3u32);
    })?;
    println!("{n}");

    Ok(())
}


