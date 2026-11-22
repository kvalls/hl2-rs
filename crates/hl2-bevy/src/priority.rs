//! Thread scheduling priority, declared once for every thread the game runs: the main thread
//! when it starts, and each Bevy task pool thread as it spawns (every launch builds its plugins
//! with [`task_pools`]). Frame work (the main thread and the compute pool that runs systems in
//! parallel) asks for the highest interactive priority; asset loading and background workers ask
//! for one step lower, so they never delay a frame.
//!
//! macOS: quality-of-service classes, which also decide whether a thread may use the
//! performance cores under load. Other platforms keep their default priorities (Windows is a
//! backlog item). The application's Info.plist adds Game Mode and opts out of App Nap.
use bevy::app::{TaskPoolOptions, TaskPoolPlugin, TaskPoolThreadAssignmentPolicy};
use std::sync::Arc;

/// What a thread does, from the scheduler's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// Work a displayed frame waits for.
    Frame,
    /// Work no frame waits for (asset loading, map preparation, file IO).
    Background,
}

/// Gives the calling thread `priority`. Never fails a launch: a refusal keeps the default.
pub fn set_current_thread(priority: Priority) {
    #[cfg(target_os = "macos")]
    {
        // <sys/qos.h>: QOS_CLASS_USER_INTERACTIVE and QOS_CLASS_USER_INITIATED.
        let class = match priority {
            Priority::Frame => 0x21,
            Priority::Background => 0x19,
        };
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
        }
        // SAFETY: a libSystem call that only changes the calling thread's scheduling class.
        let _ = unsafe { pthread_set_qos_class_self_np(class, 0) };
    }
    #[cfg(not(target_os = "macos"))]
    let _ = priority;
}

/// Bevy's task pools with each thread's priority set as it spawns.
pub fn task_pools() -> TaskPoolPlugin {
    let defaults = TaskPoolOptions::default();
    let with = |policy: TaskPoolThreadAssignmentPolicy, priority: Priority| {
        TaskPoolThreadAssignmentPolicy {
            on_thread_spawn: Some(Arc::new(move || set_current_thread(priority))),
            ..policy
        }
    };
    TaskPoolPlugin {
        task_pool_options: TaskPoolOptions {
            compute: with(defaults.compute, Priority::Frame),
            async_compute: with(defaults.async_compute, Priority::Background),
            io: with(defaults.io, Priority::Background),
            ..defaults
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Priority, task_pools};

    #[test]
    fn every_pool_sets_its_threads_priority_and_keeps_bevys_sizes() {
        let pools = task_pools().task_pool_options;
        let defaults = bevy::app::TaskPoolOptions::default();
        for (pool, default) in [
            (&pools.compute, &defaults.compute),
            (&pools.async_compute, &defaults.async_compute),
            (&pools.io, &defaults.io),
        ] {
            assert!(pool.on_thread_spawn.is_some());
            assert_eq!(
                (pool.min_threads, pool.max_threads, pool.percent),
                (default.min_threads, default.max_threads, default.percent)
            );
        }
        // Setting a priority on a test thread is harmless and never panics.
        super::set_current_thread(Priority::Background);
    }

    /// The scheduler reports the class each priority asked for.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_threads_take_the_requested_quality_of_service() {
        unsafe extern "C" {
            fn pthread_self() -> usize;
            fn pthread_get_qos_class_np(thread: usize, class: *mut u32, relative: *mut i32) -> i32;
        }
        for (priority, expected) in [(Priority::Frame, 0x21), (Priority::Background, 0x19)] {
            let class = std::thread::spawn(move || {
                super::set_current_thread(priority);
                let (mut class, mut relative) = (0u32, 0i32);
                // SAFETY: queries the calling thread's own class into two local integers.
                let status =
                    unsafe { pthread_get_qos_class_np(pthread_self(), &mut class, &mut relative) };
                assert_eq!(status, 0);
                class
            })
            .join()
            .unwrap();
            assert_eq!(class, expected, "{priority:?}");
        }
    }
}
