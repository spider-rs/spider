use std::process::{Command};
pub mod node_crawler;
pub mod go_crolly;

/// build executables for bench marks
pub fn main() {
    node_crawler::gen_crawl();
    go_crolly::gen_crawl();

    // install go deps
    Command::new("go").arg("mod").arg("init").arg("example.com/gospider").output().expect("go init failed");
    Command::new("go").arg("get").arg("-u").arg("github.com/gocolly/colly/v2").output().expect("go get colly failed");
    Command::new("go").arg("mod").arg("tidy").output().expect("go tidy failed");
    Command::new("go").arg("build").output().expect("go build failed");
    // install node deps
    Command::new("npm").arg("init").arg("-y").output().expect("go init failed");
    Command::new("npm").arg("i").arg("crawler").arg("--save").output().expect("go init failed");

    if cfg!(target_os = "linux") {
        Command::new("apt-get").arg("install").arg("wget").output().expect("wget install failed");
    }
}
