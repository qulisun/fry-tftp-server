//! Windows Event Log integration via tracing layer.
//!
//! Writes tracing events to the Windows Application Event Log using
//! the ReportEventW API. Events are tagged with the source name
//! "Fry TFTP Server".

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use tracing::field::{Field, Visit};
use tracing::{Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::EventLog::{
    DeregisterEventSource, RegisterEventSourceW, ReportEventW, EVENTLOG_ERROR_TYPE,
    EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE,
};

/// A tracing layer that writes events to the Windows Event Log.
pub struct EventLogLayer {
    handle: HANDLE,
}

// SAFETY: The event log handle is thread-safe (ReportEventW is thread-safe).
unsafe impl Send for EventLogLayer {}
unsafe impl Sync for EventLogLayer {}

impl EventLogLayer {
    /// Create a new EventLogLayer that writes to the Application log
    /// under the given source name.
    pub fn new(source_name: &str) -> Result<Self, std::io::Error> {
        let wide: Vec<u16> = OsStr::new(source_name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe { RegisterEventSourceW(std::ptr::null(), wide.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }

        Ok(Self { handle })
    }
}

impl Drop for EventLogLayer {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                DeregisterEventSource(self.handle);
            }
        }
    }
}

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for EventLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let level = *event.metadata().level();

        // Map tracing level to Windows event type
        let event_type = match level {
            Level::ERROR => EVENTLOG_ERROR_TYPE,
            Level::WARN => EVENTLOG_WARNING_TYPE,
            _ => EVENTLOG_INFORMATION_TYPE,
        };

        // Only log WARN and above to event log to avoid flooding
        if level > Level::WARN {
            return;
        }

        // Collect the message
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let message = format!(
            "[{}] {}: {}",
            level,
            event.metadata().target(),
            visitor.message
        );

        let wide_message: Vec<u16> = OsStr::new(&message)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let strings = [wide_message.as_ptr()];

        unsafe {
            ReportEventW(
                self.handle,
                event_type,
                0,    // category
                1000, // event ID
                std::ptr::null_mut(),
                1, // number of strings
                0, // data size
                strings.as_ptr(),
                std::ptr::null_mut(),
            );
        }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else if !self.message.is_empty() {
            self.message
                .push_str(&format!(" {}={:?}", field.name(), value));
        } else {
            self.message = format!("{}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if !self.message.is_empty() {
            self.message
                .push_str(&format!(" {}={}", field.name(), value));
        } else {
            self.message = format!("{}={}", field.name(), value);
        }
    }
}
