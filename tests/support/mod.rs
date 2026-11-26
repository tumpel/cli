use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub async fn serve(answers: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut lines = Vec::new();
        for answer in answers {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(stream.read_u8().await.unwrap());
            }
            let head = String::from_utf8(head).unwrap();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap_or("0")
                .parse()
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            let line = head.lines().next().unwrap();
            lines.push(match String::from_utf8(body).unwrap() {
                body if body.is_empty() => line.to_string(),
                body => format!("{line} {body}"),
            });
            stream.write_all(answer.as_bytes()).await.unwrap();
        }
        lines
    });
    (url, task)
}
