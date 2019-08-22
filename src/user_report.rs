use crate::command::User;
use futures::Future;
use log::Level;
use std::error::Error;

#[cfg(feature = "with-sentry")]
mod with_sentry {
    use super::User;
    use failure::AsFail;
    use sentry::integrations::failure::FailureHubExt;
    use sentry::{Hub, Level};
    use serde::Serialize;
    use std::error::Error;
    use std::sync::Arc;

    const SENTRY_DSN: &str = "https://9a800ab4e1084d528c7dd2bfcf3e3a06@talkback.golem.network/7";

    #[derive(Clone)]
    pub struct UserReportHandle(Option<Arc<Hub>>);

    impl<UserRef: AsRef<User>> From<UserRef> for UserReportHandle {
        fn from(u: UserRef) -> Self {
            UserReportHandle(Some(new_hub(u.as_ref())))
        }
    }

    impl UserReportHandle {
        #[inline]
        pub fn start(user: &Option<User>) -> UserReportHandle {
            UserReportHandle(user.as_ref().map(new_hub))
        }

        pub fn empty() -> UserReportHandle {
            UserReportHandle(None)
        }

        pub fn new_context(&self) -> Self {
            if let Some(hub) = &self.0 {
                UserReportHandle(Some(Arc::new(Hub::new_from_top(hub))))
            } else {
                UserReportHandle(None)
            }
        }
    }

    fn new_hub_clean(user: &User) -> Arc<Hub> {
        use sentry::*;

        let client = sentry::Hub::with(|h| h.client());

        if client.is_some() {
            return Arc::new(Hub::new_from_top(Hub::current()));
        }

        log::info!("new client!");
        let release = Some(crate::version::PACKAGE_VERSION.into());
        let environment = Some(user.env.to_str());
        let node_id = user.id.clone();

        let client: Arc<Client> = Arc::new(
            (
                SENTRY_DSN,
                ClientOptions {
                    release,
                    environment,
                    ..ClientOptions::default()
                },
            )
                .into(),
        );

        sentry::Hub::with(|h| {
            h.bind_client(Some(client.clone()));
            h.configure_scope(|s| {
                s.set_user(Some(sentry::protocol::User {
                    id: Some(node_id),
                    ..sentry::protocol::User::default()
                }))
            })
        });

        Arc::new(Hub::new_from_top(Hub::current()))
    }

    pub fn new_hub(user: &User) -> Arc<Hub> {
        let h = new_hub_clean(user);

        h.configure_scope(|s| {
            s.set_user(Some(sentry::User {
                id: Some(user.id.clone().into()),
                username: user.node_name.as_ref().and_then(|n| n.clone().into()),
                ..sentry::User::default()
            }));

            if let Some(ref gv) = &user.golem_version {
                s.set_tag("golem_version".into(), gv.to_string());
            }
        });

        h
    }

    fn map_log_level(log_level: log::Level) -> sentry::Level {
        match log_level {
            log::Level::Trace | log::Level::Debug => sentry::Level::Debug,
            log::Level::Info => sentry::Level::Info,
            log::Level::Warn => sentry::Level::Warning,
            log::Level::Error => sentry::Level::Error,
        }
    }

    impl UserReportHandle {
        pub fn emit_error(&self, stage: &'static str, error: &(dyn Error + 'static)) {
            log::error!("failed processing {}: {}", stage, error);
            if let Some(hub) = &self.0 {
                hub.capture_message(
                    &format!("failed processing {}: {}", stage, error),
                    Level::Error,
                );
            }
        }

        pub fn emit_fail(&self, e: &impl AsFail) {
            if let Some(hub) = &self.0 {
                let _ = hub.capture_fail(e.as_fail());
            }
        }

        pub fn emit_warn(&self, message: String) {
            if let Some(hub) = &self.0 {
                let _uid = hub.capture_message(&message, Level::Warning);
            }
        }

        pub fn add_breadcrumb<MessageFactory: FnOnce() -> String>(
            &self,
            level: log::Level,
            message_factory: MessageFactory,
        ) {
            if let Some(hub) = &self.0 {
                let mut b = sentry::protocol::Breadcrumb::default();
                b.message = Some(message_factory());
                b.level = map_log_level(level);
                hub.add_breadcrumb(b);
            }
        }

        pub fn annotate(&self, key: &str, value: &impl Serialize) {
            if let Some(hub) = &self.0 {
                hub.configure_scope(|s| {
                    s.set_extra(key, serde_json::to_value(value).unwrap());
                })
            }
        }
    }

    pub fn init() {
        sentry::integrations::panic::register_panic_handler()
    }
}

#[cfg(not(feature = "with-sentry"))]
mod without_sentry {
    use super::User;
    use failure::AsFail;
    use log::Level;
    use serde::Serialize;
    use std::error::Error;

    #[derive(Clone)]
    pub struct UserReportHandle {}

    impl<UserRef: AsRef<User>> From<UserRef> for UserReportHandle {
        fn from(_u: UserRef) -> Self {
            UserReportHandle {}
        }
    }

    impl UserReportHandle {
        #[inline]
        pub fn start(_user: &Option<User>) -> UserReportHandle {
            UserReportHandle {}
        }

        #[inline]
        pub fn empty() -> UserReportHandle {
            UserReportHandle {}
        }

        pub fn new_context(&self) -> Self {
            UserReportHandle::empty()
        }
    }

    impl UserReportHandle {
        pub fn emit_error(&self, stage: &'static str, error: &(dyn Error + 'static)) {
            log::error!("failed processing {}: {}", stage, error);
        }

        pub fn emit_fail(&self, _e: &impl AsFail) {}

        pub fn emit_warn(&self, _message: String) {}

        #[inline]
        pub fn add_breadcrumb<MessageFactory: FnOnce() -> String>(
            &self,
            _level: Level,
            _message_factory: MessageFactory,
        ) {

        }

        pub fn annotate(&self, _key: &str, _value: &impl Serialize) {}
    }

    pub fn init() {}
}

#[cfg(feature = "with-sentry")]
pub use with_sentry::{init, UserReportHandle};

#[cfg(not(feature = "with-sentry"))]
pub use without_sentry::{init, UserReportHandle};

impl UserReportHandle {
    #[inline]
    pub fn wrap_future<F: Future>(
        &self,
        stage: &'static str,
        f: F,
    ) -> impl Future<Item = F::Item, Error = F::Error>
    where
        F::Error: Error + 'static,
    {
        let reporter = self.clone();

        f.map_err(move |e| {
            reporter.emit_error(stage, &e);
            e
        })
    }

    #[inline]
    pub fn add_note<F: FnOnce() -> String>(&self, message_factory: F) {
        self.add_breadcrumb(Level::Info, message_factory);
    }

    #[inline]
    pub fn add_err<MessageFactory: FnOnce() -> String>(&self, message_factory: MessageFactory) {
        self.add_breadcrumb(Level::Error, message_factory);
    }
}
