//! Releasing physical memory on Windows: trimming every working set and
//! flushing or purging the modified and standby lists, the way PCL2 and
//! Mem Reduct do. Each step needs an administrator token, so an app that is not
//! elevated runs one elevated copy of itself per optimization (see [`ARG`]).
use serde::{Deserialize, Serialize};

/// Starts the elevated copy: `--optimize-memory <area mask> <result file>`.
pub const ARG: &str = "--optimize-memory";
/// The elevated copy only writes a new file named like this.
#[cfg_attr(not(windows), allow(dead_code))]
const RESULT_PREFIX: &str = "agentharbor-memory-";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryArea {
    WorkingSets,
    SystemFileCache,
    ModifiedFileCache,
    ModifiedList,
    StandbyList,
    LowPriorityStandbyList,
    RegistryCache,
    CombinePages,
}

impl MemoryArea {
    /// In the order they run: trimmed working sets land on the modified and
    /// standby lists, which the later steps write out and purge.
    pub const ALL: [Self; 8] = [
        Self::WorkingSets,
        Self::SystemFileCache,
        Self::ModifiedFileCache,
        Self::ModifiedList,
        Self::StandbyList,
        Self::LowPriorityStandbyList,
        Self::RegistryCache,
        Self::CombinePages,
    ];

    /// Writing out the whole modified list or purging the whole standby list
    /// can stall the system while it rereads what it had cached: opt-in.
    pub fn on_by_default(self) -> bool {
        !matches!(self, Self::ModifiedList | Self::StandbyList)
    }

    fn bit(self) -> u32 {
        1 << self as u32
    }
}

pub fn default_areas() -> Vec<MemoryArea> {
    MemoryArea::ALL
        .into_iter()
        .filter(|area| area.on_by_default())
        .collect()
}

/// Saved names this version knows, in run order; unknown ones are dropped.
pub fn known_areas(names: &[String]) -> Vec<MemoryArea> {
    let chosen: Vec<MemoryArea> = names
        .iter()
        .filter_map(|name| serde_json::from_value(serde_json::Value::String(name.clone())).ok())
        .collect();
    from_mask(mask(&chosen))
}

pub fn area_names(areas: &[MemoryArea]) -> Vec<String> {
    areas
        .iter()
        .filter_map(|area| serde_json::to_value(area).ok()?.as_str().map(str::to_owned))
        .collect()
}

fn mask(areas: &[MemoryArea]) -> u32 {
    areas.iter().fold(0, |mask, area| mask | area.bit())
}

fn from_mask(mask: u32) -> Vec<MemoryArea> {
    MemoryArea::ALL
        .into_iter()
        .filter(|area| mask & area.bit() != 0)
        .collect()
}

/// Physical memory by page list, in bytes, as Resource Monitor splits it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshot {
    pub total: u64,
    pub in_use: u64,
    pub modified: u64,
    pub standby: u64,
    pub free: u64,
}

impl MemorySnapshot {
    /// Pages taken off processes: what trimming working sets gives back.
    pub fn released_since(&self, before: &Self) -> u64 {
        before.in_use.saturating_sub(self.in_use)
    }

    /// Pages handed back to the free list: what purging the standby list gives back.
    pub fn freed_since(&self, before: &Self) -> u64 {
        self.free.saturating_sub(before.free)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatus {
    pub supported: bool,
    /// The app holds an administrator token: optimizing asks nothing.
    pub elevated: bool,
    pub memory: Option<MemorySnapshot>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryOptimizeOutcome {
    pub before: MemorySnapshot,
    pub after: MemorySnapshot,
    /// Steps Windows refused, such as one this version does not have.
    pub failed: Vec<MemoryArea>,
}

pub fn status() -> MemoryStatus {
    MemoryStatus {
        supported: cfg!(windows),
        elevated: imp::is_elevated(),
        memory: imp::snapshot(),
    }
}

static RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Runs the chosen steps, here when elevated or in an elevated copy otherwise.
/// Blocks until Windows' consent prompt is answered and the steps are done.
pub fn optimize(areas: &[MemoryArea]) -> Result<MemoryOptimizeOutcome, String> {
    use std::sync::atomic::Ordering;
    if areas.is_empty() {
        return Err("maintenance.memory.errors.noAreas".into());
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        return Err("maintenance.memory.errors.busy".into());
    }
    let areas = &from_mask(mask(areas));
    let outcome = if imp::is_elevated() {
        imp::optimize_here(areas).ok_or_else(|| "maintenance.memory.errors.failed".into())
    } else {
        imp::optimize_elevated(areas)
    };
    RUNNING.store(false, Ordering::Release);
    outcome
}

/// The elevated copy's whole run, before any app setup; `None` for a normal start.
pub fn run_from_args() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != ARG {
        return None;
    }
    let (Some(mask), Some(result)) = (args.next(), args.next()) else {
        return Some(2);
    };
    let Some(mask) = mask.to_str().and_then(|mask| mask.parse().ok()) else {
        return Some(2);
    };
    Some(imp::child(mask, std::path::Path::new(&result)))
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use std::io::Write;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_CANCELLED, ERROR_NOT_ALL_ASSIGNED, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE, LUID, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, GetTokenInformation, LookupPrivilegeValueW, TokenElevation,
        LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION,
        TOKEN_PRIVILEGES, TOKEN_QUERY,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, FlushFileBuffers,
        GetDriveTypeW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::SystemInformation::{
        GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    // SYSTEM_INFORMATION_CLASS values ntdll takes for these steps.
    const SYSTEM_MEMORY_LIST_INFORMATION: i32 = 80;
    const SYSTEM_FILE_CACHE_INFORMATION_EX: i32 = 81;
    const SYSTEM_COMBINE_PHYSICAL_MEMORY_INFORMATION: i32 = 130;
    const SYSTEM_REGISTRY_RECONCILIATION_INFORMATION: i32 = 155;
    // SYSTEM_MEMORY_LIST_COMMAND
    const MEMORY_EMPTY_WORKING_SETS: i32 = 2;
    const MEMORY_FLUSH_MODIFIED_LIST: i32 = 3;
    const MEMORY_PURGE_STANDBY_LIST: i32 = 4;
    const MEMORY_PURGE_LOW_PRIORITY_STANDBY_LIST: i32 = 5;
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    /// The consent prompt can sit unanswered; the steps themselves take seconds.
    const CHILD_TIMEOUT_MS: u32 = 5 * 60 * 1000;

    #[repr(C)]
    #[derive(Default)]
    struct SystemMemoryListInformation {
        zero_page_count: usize,
        free_page_count: usize,
        modified_page_count: usize,
        modified_no_write_page_count: usize,
        bad_page_count: usize,
        page_count_by_priority: [usize; 8],
        repurposed_pages_by_priority: [usize; 8],
        modified_page_count_page_file: usize,
    }

    #[repr(C)]
    #[derive(Default)]
    struct SystemFileCacheInformation {
        current_size: usize,
        peak_size: usize,
        page_fault_count: u32,
        minimum_working_set: usize,
        maximum_working_set: usize,
        current_size_including_transition_in_pages: usize,
        peak_size_including_transition_in_pages: usize,
        transition_repurpose_count: u32,
        flags: u32,
    }

    #[repr(C)]
    struct MemoryCombineInformationEx {
        handle: HANDLE,
        pages_combined: usize,
        flags: u32,
    }

    #[link(name = "ntdll", kind = "raw-dylib")]
    unsafe extern "system" {
        fn NtQuerySystemInformation(
            class: i32,
            info: *mut c_void,
            length: u32,
            returned: *mut u32,
        ) -> i32;
        fn NtSetSystemInformation(class: i32, info: *mut c_void, length: u32) -> i32;
    }

    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    pub fn is_elevated() -> bool {
        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut length = 0;
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut length,
            )
        };
        unsafe { CloseHandle(token) };
        ok != 0 && elevation.TokenIsElevated != 0
    }

    pub fn snapshot() -> Option<MemorySnapshot> {
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
            return None;
        }
        let total = status.ullTotalPhys;
        let mut lists = SystemMemoryListInformation::default();
        let queried = unsafe {
            NtQuerySystemInformation(
                SYSTEM_MEMORY_LIST_INFORMATION,
                (&mut lists as *mut SystemMemoryListInformation).cast(),
                size_of::<SystemMemoryListInformation>() as u32,
                ptr::null_mut(),
            )
        } >= 0;
        if !queried {
            // Only the available total: no split into standby and free.
            return Some(MemorySnapshot {
                total,
                in_use: total.saturating_sub(status.ullAvailPhys),
                free: status.ullAvailPhys,
                ..MemorySnapshot::default()
            });
        }
        let mut info: SYSTEM_INFO = unsafe { std::mem::zeroed() };
        unsafe { GetSystemInfo(&mut info) };
        let page = u64::from(info.dwPageSize.max(4096));
        let pages = |count: usize| count as u64 * page;
        let free = pages(lists.zero_page_count + lists.free_page_count);
        let standby = pages(lists.page_count_by_priority.iter().sum());
        let modified = pages(lists.modified_page_count + lists.modified_no_write_page_count);
        Some(MemorySnapshot {
            total,
            in_use: total.saturating_sub(free + standby + modified),
            modified,
            standby,
            free,
        })
    }

    pub fn optimize_here(areas: &[MemoryArea]) -> Option<MemoryOptimizeOutcome> {
        let before = snapshot()?;
        enable_privileges();
        let failed = areas.iter().copied().filter(|&area| !run(area)).collect();
        Some(MemoryOptimizeOutcome {
            before,
            after: snapshot()?,
            failed,
        })
    }

    pub fn child(mask: u32, result: &Path) -> i32 {
        let named = result
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(RESULT_PREFIX) && name.ends_with(".json"));
        if !named || !is_elevated() {
            return 2;
        }
        let Some(outcome) = optimize_here(&from_mask(mask)) else {
            return 3;
        };
        let written = serde_json::to_vec(&outcome).ok().is_some_and(|json| {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(result)
                .and_then(|mut file| file.write_all(&json))
                .is_ok()
        });
        if written {
            0
        } else {
            3
        }
    }

    pub fn optimize_elevated(areas: &[MemoryArea]) -> Result<MemoryOptimizeOutcome, String> {
        let failed = || "maintenance.memory.errors.failed".to_owned();
        let exe = std::env::current_exe().map_err(|_| failed())?;
        let result =
            std::env::temp_dir().join(format!("{RESULT_PREFIX}{}.json", uuid::Uuid::new_v4()));
        let parameters = format!("{ARG} {} \"{}\"", mask(areas), result.display());
        let (verb, file, parameters) = (
            wide("runas".as_ref()),
            wide(exe.as_os_str()),
            wide(parameters.as_ref()),
        );
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = parameters.as_ptr();
        info.nShow = SW_HIDE;
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            return Err(if unsafe { GetLastError() } == ERROR_CANCELLED {
                "maintenance.memory.errors.cancelled".into()
            } else {
                "maintenance.memory.errors.elevate".into()
            });
        }
        if info.hProcess.is_null() {
            return Err("maintenance.memory.errors.elevate".into());
        }
        let finished =
            unsafe { WaitForSingleObject(info.hProcess, CHILD_TIMEOUT_MS) } == WAIT_OBJECT_0;
        let mut code = 1;
        unsafe {
            GetExitCodeProcess(info.hProcess, &mut code);
            CloseHandle(info.hProcess);
        }
        let outcome = std::fs::read(&result)
            .ok()
            .and_then(|json| serde_json::from_slice(&json).ok());
        let _ = std::fs::remove_file(&result);
        match outcome {
            Some(outcome) if finished && code == 0 => Ok(outcome),
            _ => Err(failed()),
        }
    }

    /// The memory list commands need the first, the file cache the second.
    fn enable_privileges() {
        let mut token: HANDLE = ptr::null_mut();
        let access = TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY;
        if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut token) } == 0 {
            return;
        }
        for name in [
            "SeProfileSingleProcessPrivilege",
            "SeIncreaseQuotaPrivilege",
        ] {
            let name = wide(name.as_ref());
            let mut luid = LUID {
                LowPart: 0,
                HighPart: 0,
            };
            if unsafe { LookupPrivilegeValueW(ptr::null(), name.as_ptr(), &mut luid) } == 0 {
                continue;
            }
            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            let adjusted = unsafe {
                AdjustTokenPrivileges(token, 0, &privileges, 0, ptr::null_mut(), ptr::null_mut())
            };
            if adjusted == 0 || unsafe { GetLastError() } == ERROR_NOT_ALL_ASSIGNED {
                log::warn!("memory optimization could not enable a privilege");
            }
        }
        unsafe { CloseHandle(token) };
    }

    fn set<T>(class: i32, info: &mut T) -> bool {
        unsafe {
            NtSetSystemInformation(class, (info as *mut T).cast(), size_of::<T>() as u32) >= 0
        }
    }

    fn list_command(mut command: i32) -> bool {
        set(SYSTEM_MEMORY_LIST_INFORMATION, &mut command)
    }

    fn run(area: MemoryArea) -> bool {
        match area {
            MemoryArea::WorkingSets => list_command(MEMORY_EMPTY_WORKING_SETS),
            MemoryArea::SystemFileCache => {
                // Both limits at SIZE_T's maximum: trim the cache's working set.
                let mut info = SystemFileCacheInformation {
                    minimum_working_set: usize::MAX,
                    maximum_working_set: usize::MAX,
                    ..Default::default()
                };
                set(SYSTEM_FILE_CACHE_INFORMATION_EX, &mut info)
            }
            MemoryArea::ModifiedFileCache => flush_volumes(),
            MemoryArea::ModifiedList => list_command(MEMORY_FLUSH_MODIFIED_LIST),
            MemoryArea::StandbyList => list_command(MEMORY_PURGE_STANDBY_LIST),
            MemoryArea::LowPriorityStandbyList => {
                list_command(MEMORY_PURGE_LOW_PRIORITY_STANDBY_LIST)
            }
            MemoryArea::RegistryCache => unsafe {
                NtSetSystemInformation(
                    SYSTEM_REGISTRY_RECONCILIATION_INFORMATION,
                    ptr::null_mut(),
                    0,
                ) >= 0
            },
            MemoryArea::CombinePages => {
                let mut info = MemoryCombineInformationEx {
                    handle: ptr::null_mut(),
                    pages_combined: 0,
                    flags: 0,
                };
                set(SYSTEM_COMBINE_PHYSICAL_MEMORY_INFORMATION, &mut info)
            }
        }
    }

    /// Writes every local volume's cached changes to disk, so the file cache
    /// and modified list have less left that must stay in memory.
    fn flush_volumes() -> bool {
        let mut name = [0u16; 64];
        let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
        if find == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut flushed = false;
        loop {
            // `\\?\Volume{…}\`: the drive type wants the slash, opening the volume does not.
            if matches!(
                unsafe { GetDriveTypeW(name.as_ptr()) },
                DRIVE_FIXED | DRIVE_REMOVABLE
            ) {
                let end = name
                    .iter()
                    .position(|&unit| unit == 0)
                    .unwrap_or(name.len());
                let mut path = name[..end].to_vec();
                if path.last() == Some(&u16::from(b'\\')) {
                    path.pop();
                }
                path.push(0);
                let volume = unsafe {
                    CreateFileW(
                        path.as_ptr(),
                        GENERIC_WRITE,
                        FILE_SHARE_READ | FILE_SHARE_WRITE,
                        ptr::null(),
                        OPEN_EXISTING,
                        0,
                        ptr::null_mut(),
                    )
                };
                // Locked or empty drives do not open; they have nothing cached.
                if volume != INVALID_HANDLE_VALUE {
                    flushed |= unsafe { FlushFileBuffers(volume) } != 0;
                    unsafe { CloseHandle(volume) };
                }
            }
            if unsafe { FindNextVolumeW(find, name.as_mut_ptr(), name.len() as u32) } == 0 {
                break;
            }
        }
        unsafe { FindVolumeClose(find) };
        flushed
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn is_elevated() -> bool {
        false
    }

    pub fn snapshot() -> Option<MemorySnapshot> {
        None
    }

    pub fn optimize_here(_areas: &[MemoryArea]) -> Option<MemoryOptimizeOutcome> {
        None
    }

    pub fn optimize_elevated(_areas: &[MemoryArea]) -> Result<MemoryOptimizeOutcome, String> {
        Err("maintenance.memory.errors.unsupported".into())
    }

    pub fn child(_mask: u32, _result: &std::path::Path) -> i32 {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stall_prone_lists_are_opt_in_and_areas_run_in_order() {
        assert_eq!(
            default_areas(),
            [
                MemoryArea::WorkingSets,
                MemoryArea::SystemFileCache,
                MemoryArea::ModifiedFileCache,
                MemoryArea::LowPriorityStandbyList,
                MemoryArea::RegistryCache,
                MemoryArea::CombinePages,
            ]
        );
        let picked = [
            MemoryArea::CombinePages,
            MemoryArea::StandbyList,
            MemoryArea::WorkingSets,
        ];
        assert_eq!(
            from_mask(mask(&picked)),
            [
                MemoryArea::WorkingSets,
                MemoryArea::StandbyList,
                MemoryArea::CombinePages
            ]
        );
    }

    #[test]
    fn saved_names_survive_a_round_trip_and_unknown_ones_are_dropped() {
        let names = area_names(&[MemoryArea::StandbyList, MemoryArea::LowPriorityStandbyList]);
        assert_eq!(names, ["standbyList", "lowPriorityStandbyList"]);
        let saved = [names, vec!["fromAFutureVersion".into()]].concat();
        assert_eq!(
            known_areas(&saved),
            [MemoryArea::StandbyList, MemoryArea::LowPriorityStandbyList]
        );
    }

    #[test]
    fn released_and_freed_never_go_negative() {
        let before = MemorySnapshot {
            total: 16,
            in_use: 10,
            modified: 1,
            standby: 4,
            free: 1,
        };
        let after = MemorySnapshot {
            total: 16,
            in_use: 7,
            modified: 0,
            standby: 1,
            free: 8,
        };
        assert_eq!(
            (after.released_since(&before), after.freed_since(&before)),
            (3, 7)
        );
        assert_eq!(
            (before.released_since(&after), before.freed_since(&after)),
            (0, 0)
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_snapshot_splits_all_physical_memory_into_its_lists() {
        let memory = status().memory.expect("memory status");
        assert!(memory.total > 0 && memory.standby + memory.free > 0);
        assert_eq!(
            memory.in_use + memory.modified + memory.standby + memory.free,
            memory.total
        );
    }

    #[test]
    fn a_normal_start_is_not_the_elevated_copy() {
        assert_eq!(run_from_args(), None);
    }
}
