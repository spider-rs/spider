use std::process::Command;

pub mod go_colly;
pub mod node_crawler;

/// build executables for benchmarks
pub fn main() {
    node_crawler::gen_crawl();
    go_colly::gen_crawl();

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
    Command::new("go")
        .args(["mod", "tidy"])
        .output()
        .expect("go tidy failed");
    Command::new("go")
        .arg("build")
        .output()
        .expect("go build failed");
    // install node deps
    Command::new("npm")
        .args(["init", "-y"])
        .output()
        .expect("npm init failed");
    Command::new("npm")
        .args(["i", "crawler", "--save"])
        .output()
        .expect("npm install crawler failed");

    // Install wget based on the operating system
    if cfg!(target_os = "linux") {
        if is_debian_based() {
            Command::new("apt-get")
                .args(["install", "-y", "wget"])
                .output()
                .expect("apt-get install wget failed");
        } else if is_rpm_based() {
            Command::new("yum")
                .args(["install", "-y", "wget"])
                .output()
                .expect("yum install wget failed");
        }
    } else if cfg!(target_os = "macos") {
        Command::new("brew")
            .args(["install", "wget"])
            .output()
            .expect("brew install wget failed");
    }
}

#[cfg(target_os = "linux")]
/// Is Debian OS?
fn is_debian_based() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("grep -Ei 'debian|buntu' /etc/*release")
        .output()
        .map_or(false, |output| output.status.success())
}

#[cfg(target_os = "linux")]
/// Is Red Hat Package Manager?
fn is_rpm_based() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("grep -Ei 'fedora|redhat|centos' /etc/*release")
        .output()
        .map_or(false, |output| output.status.success())
}

#[cfg(not(target_os = "linux"))]
/// Is Debian OS?
fn is_debian_based() -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
/// Is Red Hat Package Manager?
fn is_rpm_based() -> bool {
    false
}
