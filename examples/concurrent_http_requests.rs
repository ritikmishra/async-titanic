use async_titanic::async_executor::Executor;
use async_titanic::reactor::Reactor;
use async_titanic::store_futs_statically;
use std::time::Instant;

const REQUEST_CONTENT_1S: &'static str = "GET /delay/6 HTTP/1.1
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9
Accept-Encoding: identity
Accept-Language: en-US,en;q=0.9
Cache-Control: max-age=0
Connection: close
Host: httpbin.org
Upgrade-Insecure-Requests: 1
User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/102.0.0.0 Safari/537.36

";

const REQUEST_CONTENT_4S: &'static str = "GET /delay/10 HTTP/1.1
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9
Accept-Encoding: identity
Accept-Language: en-US,en;q=0.9
Cache-Control: max-age=0
Connection: close
Host: httpbin.org
Upgrade-Insecure-Requests: 1
User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/102.0.0.0 Safari/537.36

";

async fn send_data(data: &'static str, reactor: &'static Reactor, i: i32) {
    let sock1 = mio::net::TcpStream::connect("54.208.105.16:80".parse().unwrap()).unwrap();
    let mut sock1 = reactor.register_new_socket(sock1);

    println!("sending data (fut {i})");
    sock1.write(data.as_bytes()).await.unwrap();
    println!("sent data (fut {i})");
    let start = Instant::now();

    println!("recieving data (fut {i})");
    let data = sock1.read().await.unwrap();
    print!("{}", String::from_utf8_lossy(&data));

    println!(
        "all done! (fut {i}), took {}",
        Instant::now().duration_since(start).as_secs_f64()
    );
}

fn main() {
    // SAFETY: This is the only mutable borrow of `REACTOR` that exists, or that
    // can ever exist (since it is inside the block)
    let reactor: &'static Reactor = unsafe {
        static mut REACTOR: Option<Reactor> = None;
        REACTOR.insert(Reactor::new())
    };

    let futs = store_futs_statically!(
        600;
        send_data(REQUEST_CONTENT_1S, reactor, 1),
        send_data(REQUEST_CONTENT_4S, reactor, 2),
        send_data(REQUEST_CONTENT_4S, reactor, 3)
    );

    let mut executor = Executor::new_with_tasks(futs);
    loop {
        executor.step_once();
        reactor.tick();
        if executor.all_tasks_done() {
            break;
        }
    }
}
