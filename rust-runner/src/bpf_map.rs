use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;
use std::ptr;

use crate::stats::TaskInfo;

unsafe extern "C" {
    fn bpf_obj_get(pathname: *const libc::c_char) -> libc::c_int;
    fn bpf_map_get_next_key(
        fd: libc::c_int,
        key: *const libc::c_void,
        next_key: *mut libc::c_void,
    ) -> libc::c_int;
    fn bpf_map_lookup_elem(
        fd: libc::c_int,
        key: *const libc::c_void,
        value: *mut libc::c_void,
    ) -> libc::c_int;
}

pub fn open_pinned_map(path: &str) -> io::Result<RawFd> {
    let c_path = CString::new(path).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "path contains embedded NUL byte",
        )
    })?;

    let fd = unsafe { bpf_obj_get(c_path.as_ptr()) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd as RawFd)
    }
}

pub fn iterate_task_info(map_fd: RawFd) -> io::Result<Vec<(u32, TaskInfo)>> {
    let mut entries = Vec::new();
    let mut key: u32 = 0;
    let mut next_key: u32 = 0;
    let mut first = true;

    loop {
        let key_ptr = if first {
            ptr::null()
        } else {
            &key as *const u32
        };

        let ret = unsafe {
            bpf_map_get_next_key(
                map_fd,
                key_ptr as *const libc::c_void,
                &mut next_key as *mut u32 as *mut libc::c_void,
            )
        };

        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ENOENT) {
                break;
            } else {
                return Err(err);
            }
        }

        first = false;
        key = next_key;

        let mut value = TaskInfo::default();
        let lookup_ret = unsafe {
            bpf_map_lookup_elem(
                map_fd,
                &key as *const u32 as *const libc::c_void,
                &mut value as *mut TaskInfo as *mut libc::c_void,
            )
        };
        if lookup_ret < 0 {
            return Err(io::Error::last_os_error());
        }
        entries.push((key, value));
    }

    entries.sort_by_key(|(pid, _)| *pid);
    Ok(entries)
}
