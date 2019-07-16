use log::info;

pub static PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
static TRAVIS_COMMIT: Option<&str> = option_env!("TRAVIS_COMMIT");
static TRAVIS_BUILD_NUMBER: Option<&str> = option_env!("TRAVIS_BUILD_NUMBER");
static TRAVIS_TAG: Option<&str> = option_env!("TRAVIS_TAG");
static TRAVIS_OS_NAME: Option<&str> = option_env!("TRAVIS_OS_NAME");

pub fn startup_log() {
    info!("[STARTED] simple transfer VERSION={}", PACKAGE_VERSION);
    match (
        TRAVIS_COMMIT,
        TRAVIS_BUILD_NUMBER,
        TRAVIS_TAG,
        TRAVIS_OS_NAME,
    ) {
        (Some(commit), Some(build_number), Some(tag), Some(os)) => info!(
            "BUILD_NO={}, COMMIT={}, TAG={}, OS={}",
            build_number, commit, tag, os
        ),
        _ => (),
    }
}
