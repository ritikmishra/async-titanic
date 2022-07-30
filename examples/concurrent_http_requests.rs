use async_titanic::async_executor::Executor;
use async_titanic::reactor::Reactor;
use futures::pin_mut;
use mio::net::TcpStream;

const REQUEST_CONTENT_1S: &'static str = "GET /delay/1 HTTP/1.1
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9
Accept-Encoding: identity
Accept-Language: en-US,en;q=0.9
Cache-Control: max-age=0
Connection: close
Host: httpbin.org
Upgrade-Insecure-Requests: 1
User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/102.0.0.0 Safari/537.36

";

const REQUEST_CONTENT_4S: &'static str = "GET /delay/4 HTTP/1.1
Accept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9
Accept-Encoding: identity
Accept-Language: en-US,en;q=0.9
Cache-Control: max-age=0
Connection: close
Host: httpbin.org
Upgrade-Insecure-Requests: 1
User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/102.0.0.0 Safari/537.36

";

fn main() {
    let reactor = Reactor::new();

    let fut1 = async {
        let sock1 = TcpStream::connect("54.208.105.16:80".parse().unwrap()).unwrap();
        let mut sock1 = reactor.register_new_socket(sock1);

        println!("sending data (fut 1)");
        sock1.write(REQUEST_CONTENT_1S.as_bytes()).await.unwrap();
        println!("sent data (fut 1)");

        println!("recieving data (fut 1)");
        let data = sock1.read().await.unwrap();
        print!("{}", String::from_utf8_lossy(&data));

        println!("all done! (fut 1)");
    };

    let fut2 = async {
        let sock2 = TcpStream::connect("54.208.105.16:80".parse().unwrap()).unwrap();
        let mut sock2 = reactor.register_new_socket(sock2);

        println!("sending data (fut 2)");
        sock2.write(REQUEST_CONTENT_4S.as_bytes()).await.unwrap();
        println!("sent data (fut 2)");

        println!("recieving data (fut 2)");
        let data = sock2.read().await.unwrap();
        print!("{}", String::from_utf8_lossy(&data));

        println!("all done! (fut 2)");
    };

    pin_mut!(fut1, fut2);

    let mut executor = Executor::new_with_tasks([fut1, fut2]);
    loop {
        executor.step_once();
        reactor.tick();
        if executor.all_tasks_done() {
            break;
        }
    }
}
