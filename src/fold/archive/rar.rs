//! Narrow streaming adapter to bundled RARLAB UnRAR. The native engine only
//! tests/decompresses; it never receives an extraction destination. All writes
//! pass through the archive service's checked writer.
use super::Entry as ArchiveEntry;
use std::{ffi::CString, io::Write, os::unix::ffi::OsStrExt, path::Path};
use unrar_sys as sys;
pub struct Reader {
    handle: *const sys::Handle,
}
impl Drop for Reader {
    fn drop(&mut self) {
        unsafe {
            sys::RARCloseArchive(self.handle);
        }
    }
}
fn check(code: i32) -> anyhow::Result<()> {
    anyhow::ensure!(code == sys::ERAR_SUCCESS, "RAR parser error {code}");
    Ok(())
}
extern "C" fn refuse_prompts(
    message: sys::UINT,
    _: sys::LPARAM,
    _: sys::LPARAM,
    _: sys::LPARAM,
) -> i32 {
    if message == sys::UCM_PROCESSDATA {
        0
    } else {
        -1
    }
}
impl Reader {
    pub fn open(path: &Path, extract: bool) -> anyhow::Result<Self> {
        let name = CString::new(path.as_os_str().as_bytes())?;
        // RAR's structs are C POD; zero initializes all reserved fields.
        let mut options: sys::OpenArchiveDataEx = unsafe { std::mem::zeroed() };
        options.archive_name = name.as_ptr();
        options.open_mode = if extract {
            sys::RAR_OM_EXTRACT
        } else {
            sys::RAR_OM_LIST
        };
        options.callback = Some(refuse_prompts);
        let handle = unsafe { sys::RAROpenArchiveEx(std::ptr::addr_of_mut!(options)) };
        if handle.is_null() {
            anyhow::bail!("Cannot open RAR archive ({})", options.open_result);
        }
        let reader = Self { handle };
        check(options.open_result as i32)?;
        anyhow::ensure!(
            options.flags & (sys::ROADF_VOLUME | sys::ROADF_ENCHEADERS) == 0,
            "Encrypted or multipart archives are unsupported"
        );
        Ok(reader)
    }
    pub fn next(&mut self) -> anyhow::Result<Option<ArchiveEntry>> {
        let mut h: sys::HeaderDataEx = unsafe { std::mem::zeroed() };
        let code = unsafe { sys::RARReadHeaderEx(self.handle, std::ptr::addr_of_mut!(h)) };
        if code == sys::ERAR_END_ARCHIVE {
            return Ok(None);
        }
        check(code)?;
        anyhow::ensure!(
            h.flags & (sys::RHDF_ENCRYPTED | sys::RHDF_SPLITBEFORE | sys::RHDF_SPLITAFTER) == 0,
            "Encrypted or multipart archives are unsupported"
        );
        anyhow::ensure!(h.redir_type == 0, "RAR links are unsupported");
        let mode = h.file_attr & 0o170000;
        anyhow::ensure!(
            h.host_os != 3 || matches!(mode, 0 | 0o100000 | 0o040000),
            "RAR special files are unsupported"
        );
        anyhow::ensure!(
            h.dict_size <= 256 * 1024,
            "RAR dictionary exceeds 256 MiB limit"
        );
        let name = h
            .filename_w
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| char::from_u32(c as u32).unwrap_or('\u{fffd}'))
            .collect();
        Ok(Some(ArchiveEntry {
            name,
            bytes: Some((u64::from(h.unp_size_high) << 32) | u64::from(h.unp_size)),
            directory: h.flags & sys::RHDF_DIRECTORY != 0,
        }))
    }
    pub fn skip(&mut self) -> anyhow::Result<()> {
        check(unsafe {
            sys::RARProcessFile(
                self.handle,
                sys::RAR_SKIP,
                std::ptr::null(),
                std::ptr::null(),
            )
        })
    }
    pub fn copy(&mut self, writer: &mut dyn Write) -> anyhow::Result<()> {
        let mut context = Callback {
            writer,
            error: None,
        };
        unsafe {
            sys::RARSetCallback(
                self.handle,
                Some(stream),
                &mut context as *mut Callback<'_> as sys::LPARAM,
            );
        }
        let code = unsafe {
            sys::RARProcessFile(
                self.handle,
                sys::RAR_TEST,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        unsafe {
            sys::RARSetCallback(self.handle, Some(refuse_prompts), 0);
        }
        if let Some(e) = context.error {
            return Err(e.into());
        }
        check(code)
    }
}
struct Callback<'a> {
    writer: &'a mut dyn Write,
    error: Option<std::io::Error>,
}
extern "C" fn stream(
    message: sys::UINT,
    user: sys::LPARAM,
    data: sys::LPARAM,
    length: sys::LPARAM,
) -> i32 {
    if message != sys::UCM_PROCESSDATA || user == 0 || length < 0 {
        return -1;
    }
    // The pointer lives for RARProcessFile's synchronous callback duration.
    let context = unsafe { &mut *(user as *mut Callback<'_>) };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if length == 0 {
            return Ok(());
        }
        if data == 0 {
            return Err(std::io::Error::other("Invalid RAR output"));
        }
        context
            .writer
            .write_all(unsafe { std::slice::from_raw_parts(data as *const u8, length as usize) })
    }))
    .unwrap_or_else(|_| Err(std::io::Error::other("RAR output callback failed")));
    if let Err(e) = result {
        context.error = Some(e);
        -1
    } else {
        0
    }
}
