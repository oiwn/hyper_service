use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::Service;
use hyper::{body::Incoming as IncomingBody, Request, Response};
use tokio::net::TcpListener;

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Mutex;

mod support;
use support::TokioIo;

trait ServiceFactory {
    type ServiceType: Service<
            Request<IncomingBody>,
            Response = Response<Full<Bytes>>,
            Error = hyper::Error,
        > + Send
        + 'static;

    fn create_service(&self) -> Self::ServiceType;
}

struct StatsService {
    counter: Mutex<i32>,
}

struct StatsServiceFactory;

struct TestService;
struct TestServiceFactory;

impl ServiceFactory for StatsServiceFactory {
    type ServiceType = StatsService;

    fn create_service(&self) -> Self::ServiceType {
        StatsService::new()
    }
}

impl ServiceFactory for TestServiceFactory {
    type ServiceType = TestService;

    fn create_service(&self) -> Self::ServiceType {
        TestService::new()
    }
}

impl Service<Request<IncomingBody>> for StatsService {
    type Response = Response<Full<Bytes>>;
    type Error = hyper::Error;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<IncomingBody>) -> Self::Future {
        fn root_response() -> Result<Response<Full<Bytes>>, hyper::Error> {
            let response = "stats service";
            Ok(Response::builder()
                .body(Full::new(Bytes::from(response)))
                .unwrap())
        }
        fn mk_response(s: String) -> Result<Response<Full<Bytes>>, hyper::Error> {
            Ok(Response::builder().body(Full::new(Bytes::from(s))).unwrap())
        }

        fn posts_id_response(
            post_id: &str,
        ) -> Result<Response<Full<Bytes>>, hyper::Error> {
            let response = format!("posts: {}", post_id);
            Ok(Response::builder()
                .body(Full::new(Bytes::from(response)))
                .unwrap())
        }

        let path_segments: Vec<&str> = req.uri().path().split('/').collect();

        let res = match path_segments.as_slice() {
            [""] | ["", ""] => root_response(),
            ["", "posts"] | ["", "posts", ""] => mk_response(format!(
                "posts, of course! counter = {:?}",
                self.counter
            )),
            ["", "posts", post_id] => posts_id_response(post_id),
            // Return the 404 Not Found for other routes, and don't increment counter.
            _ => return Box::pin(async { mk_response("oh no! not found".into()) }),
        };

        if req.uri().path() != "/favicon.ico" {
            *self.counter.lock().expect("lock poisoned") += 1;
        }

        Box::pin(async { res })
    }
}

impl Service<Request<IncomingBody>> for TestService {
    type Response = Response<Full<Bytes>>;
    type Error = hyper::Error;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<IncomingBody>) -> Self::Future {
        fn ok_response(s: String) -> Result<Response<Full<Bytes>>, hyper::Error> {
            Ok(Response::builder().body(Full::new(Bytes::from(s))).unwrap())
        }

        // fn fail_response(s: String) -> Result<Response<Full<Bytes>>, hyper::Error> {}

        let res = match req.uri().path() {
            "/" => ok_response(format!("here")),
            // Return the 404 Not Found for other routes, and don't increment counter.
            _ => return Box::pin(async { ok_response("oh no! not found".into()) }),
        };

        Box::pin(async { res })
    }
}

impl StatsService {
    fn new() -> Self {
        Self {
            counter: Mutex::new(10),
        }
    }
}

impl TestService {
    fn new() -> Self {
        Self {}
    }
}

async fn run_service<F>(port: u16, service_factory: F) -> anyhow::Result<()>
where
    F: ServiceFactory + Send + 'static,
    <<F as ServiceFactory>::ServiceType as Service<
        hyper::Request<hyper::body::Incoming>,
    >>::Future: Send,
{
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();

    let listener = TcpListener::bind(&addr).await?;
    println!("Listening on http://{}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let service = service_factory.create_service();
        tokio::task::spawn(async move {
            if let Err(err) =
                http1::Builder::new().serve_connection(io, service).await
            {
                println!("Failed to serve connection: {:?}", err);
            }
        });
    }
}

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.spawn(run_service(3000, StatsServiceFactory));
    rt.spawn(run_service(3001, TestServiceFactory));

    rt.block_on(async {
        // Keep the runtime alive, or implement a shutdown signal
        tokio::signal::ctrl_c().await.unwrap();
    });
    Ok(())
}
