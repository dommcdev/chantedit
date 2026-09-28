//! Minimal binding to libqpdf's job API (`qpdfjob-c.h`), which runs a qpdf
//! command line in-process.

use std::ffi::{CString, OsStr, c_char, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::ptr;

type Handle = *mut c_void;
type LogFn = unsafe extern "C" fn(data: *const c_char, len: usize, udata: *mut c_void) -> c_int;

const LOG_DISCARD: c_int = 3;
const LOG_CUSTOM: c_int = 4;
/// Exit code for "succeeded with warnings".
const EXIT_WARNING: c_int = 3;

unsafe extern "C" {
    fn qpdfjob_init() -> Handle;
    fn qpdfjob_cleanup(job: *mut Handle);
    fn qpdfjob_set_logger(job: Handle, logger: Handle);
    fn qpdfjob_initialize_from_argv(job: Handle, argv: *const *const c_char) -> c_int;
    fn qpdfjob_run(job: Handle) -> c_int;
    fn qpdflogger_create() -> Handle;
    fn qpdflogger_cleanup(logger: *mut Handle);
    fn qpdflogger_set_info(logger: Handle, dest: c_int, f: Option<LogFn>, udata: *mut c_void);
    fn qpdflogger_set_warn(logger: Handle, dest: c_int, f: Option<LogFn>, udata: *mut c_void);
    fn qpdflogger_set_error(logger: Handle, dest: c_int, f: Option<LogFn>, udata: *mut c_void);
}

unsafe extern "C" fn collect(data: *const c_char, len: usize, udata: *mut c_void) -> c_int {
    // SAFETY: `udata` is the `Vec<u8>` owned by `run`, alive for the whole job,
    // and qpdf passes `len` valid bytes.
    unsafe {
        let buf = &mut *udata.cast::<Vec<u8>>();
        buf.extend_from_slice(std::slice::from_raw_parts(data.cast::<u8>(), len));
    }
    0
}

/// Runs `qpdf ARGS…`. On failure returns qpdf's error output.
pub fn run(args: &[&OsStr]) -> Result<(), String> {
    let args: Vec<CString> = std::iter::once(OsStr::new("qpdf"))
        .chain(args.iter().copied())
        .map(|a| CString::new(a.as_bytes()))
        .collect::<Result<_, _>>()
        .map_err(|_| "file name contains a NUL byte".to_owned())?;
    let mut argv: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
    argv.push(ptr::null());

    let mut messages: Vec<u8> = Vec::new();
    let code = unsafe {
        // SAFETY: handles are created and destroyed here, argv is
        // NULL-terminated and outlives the job, and `messages` outlives the
        // logger callbacks.
        let mut logger = qpdflogger_create();
        let udata = (&raw mut messages).cast::<c_void>();
        qpdflogger_set_info(logger, LOG_DISCARD, None, ptr::null_mut());
        qpdflogger_set_warn(logger, LOG_CUSTOM, Some(collect), udata);
        qpdflogger_set_error(logger, LOG_CUSTOM, Some(collect), udata);
        let mut job = qpdfjob_init();
        qpdfjob_set_logger(job, logger);
        let mut code = qpdfjob_initialize_from_argv(job, argv.as_ptr());
        if code == 0 {
            code = qpdfjob_run(job);
        }
        qpdfjob_cleanup(&mut job);
        qpdflogger_cleanup(&mut logger);
        code
    };
    match code {
        0 | EXIT_WARNING => Ok(()),
        _ => {
            let text = String::from_utf8_lossy(&messages);
            let text = text
                .lines()
                .map(|l| l.trim_start_matches("qpdf: "))
                .collect::<Vec<_>>()
                .join(" ");
            Err(if text.is_empty() {
                format!("qpdf failed (exit code {code})")
            } else {
                text
            })
        }
    }
}
