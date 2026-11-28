//! Thread scheduling priority, declared once for every thread the game runs: the main thread
//! when it starts, and each Bevy task pool thread as it spawns (every launch builds its plugins
//! with [`task_pools`]). Frame work (the main thread and the compute pool that runs systems in
//! parallel) asks for the highest interactive priority; asset loading and background workers ask
//! for one step lower, so they never delay a frame.
//!
//! macOS: quality-of-service classes, which also decide whether a thread may use the
//! performance cores under load. The application's Info.plist adds Game Mode and opts out of App Nap.
//!
//! Windows: frame threads join the Multimedia Class Scheduler's "Games" task and run above normal
//! priority, background threads stay normal, and [`set_process`] opts the process out of EcoQoS
//! power throttling. Game Mode itself is Windows' own decision (no API opts in). Other platforms
//! keep their default priorities.
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
    #[cfg(windows)]
    windows::set_thread(priority);
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = priority;
}

/// Process-wide scheduling settings, once at startup before the first thread priority. Never fails
/// a launch: a refusal keeps the default.
pub fn set_process() {
    #[cfg(windows)]
    windows::opt_out_of_power_throttling();
}

#[cfg(windows)]
mod windows {
    use super::Priority;
    use std::ffi::c_void;

    /// `PROCESS_POWER_THROTTLING_STATE` (processthreadsapi.h).
    #[repr(C)]
    struct PowerThrottlingState {
        version: u32,
        control_mask: u32,
        state_mask: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut c_void;
        fn GetCurrentProcess() -> *mut c_void;
        fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
        fn SetProcessInformation(
            process: *mut c_void,
            class: i32,
            info: *const c_void,
            size: u32,
        ) -> i32;
    }
    #[link(name = "avrt")]
    unsafe extern "system" {
        fn AvSetMmThreadCharacteristicsW(task: *const u16, index: *mut u32) -> *mut c_void;
    }
    const THREAD_PRIORITY_NORMAL: i32 = 0;
    const THREAD_PRIORITY_ABOVE_NORMAL: i32 = 1;
    /// `ProcessPowerThrottling` in `PROCESS_INFORMATION_CLASS`.
    const PROCESS_POWER_THROTTLING: i32 = 4;
    const PROCESS_POWER_THROTTLING_CURRENT_VERSION: u32 = 1;
    const PROCESS_POWER_THROTTLING_EXECUTION_SPEED: u32 = 1;

    pub(super) fn set_thread(priority: Priority) {
        let level = match priority {
            Priority::Frame => {
                let task: Vec<u16> = "Games\0".encode_utf16().collect();
                let mut index = 0u32;
                // SAFETY: a NUL-terminated UTF-16 task name and a local index; the returned handle
                // is kept for the thread's life (MMCSS ends the registration when the thread exits).
                let _ = unsafe { AvSetMmThreadCharacteristicsW(task.as_ptr(), &mut index) };
                THREAD_PRIORITY_ABOVE_NORMAL
            }
            Priority::Background => THREAD_PRIORITY_NORMAL,
        };
        // SAFETY: the pseudo-handle of the calling thread and a documented priority level.
        let _ = unsafe { SetThreadPriority(GetCurrentThread(), level) };
    }

    pub(super) fn opt_out_of_power_throttling() {
        // Control execution-speed throttling and leave it off (the documented opt-out).
        let state = PowerThrottlingState {
            version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            control_mask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            state_mask: 0,
        };
        // SAFETY: the current-process pseudo-handle and a correctly sized, live local struct.
        let _ = unsafe {
            SetProcessInformation(
                GetCurrentProcess(),
                PROCESS_POWER_THROTTLING,
                (&raw const state).cast(),
                size_of::<PowerThrottlingState>() as u32,
            )
        };
    }
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
        super::set_process();
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
