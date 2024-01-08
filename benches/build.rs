use std::process::Command;

pub mod go_crolly;
pub mod node_crawler;

/// build executables for bench marks
pub fn main() {
    node_crawler::gen_crawl();
    go_crolly::gen_crawl();

    // install spider_worker
    Command::new("cargo")
        .args(["install", "spider_worker"])
        .output()
        .expect("cargo install spider_worker failed");
    // install go deps
    Command::new("go")
        .args(["mod", "init", "example.com/gospider"])
        .output()
        .expect("go init failed");
    Command::new("go")
        .args(["get", "-u", "github.com/gocolly/colly/v2"])
        .output()
        .expect("go get colly failed");
    Command::new("go").args(["mod", "tidy"]).output().expect(
        "go tidy failed",
    );
    Command::new("go").arg("build").output().expect(
        "go build failed",
    );
    // install node deps
    Command::new("npm").args(["init", "-y"]).output().expect(
        "npm init failed",
    );
    Command::new("npm")
        .args(["i", "crawler", "--save"])
        .output()
        .expect("npm install crawler failed");

    if cfg!(target_os = "linux") {
        Command::new("apt-get")
            .args(["install", "wget"])
            .output()
            .expect("wget install failed");
    }
}
