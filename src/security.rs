use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::os::windows::process::CommandExt;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_SUCCESS, HANDLE, LUID,
};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
    GRANT_ACCESS, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, TRUSTEE_IS_SID,
    TRUSTEE_IS_WELL_KNOWN_GROUP,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, ACL, DACL_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    UNPROTECTED_DACL_SECURITY_INFORMATION,
};
use windows_sys::Win32::Storage::FileSystem::{
    GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_READONLY,
    INVALID_FILE_ATTRIBUTES,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::color::encode_wide;

const SID_ADMINISTRATORS: &str = "S-1-5-32-544";
const SID_TRUSTED_INSTALLER: &str =
    "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";

const SUB_CONTAINERS_AND_OBJECTS_INHERIT: u32 = 3;
const GENERIC_ALL: u32 = 0x10000000;
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[link(name = "advapi32")]
unsafe extern "system" {
    fn ConvertStringSidToSidW(
        string_sid: *const u16,
        sid: *mut *mut std::ffi::c_void,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(hmem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

struct AutoSid(*mut std::ffi::c_void);

impl Drop for AutoSid {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

fn get_sid(sid_str: &str) -> Option<AutoSid> {
    let wide = encode_wide(sid_str);
    let mut psid: *mut std::ffi::c_void = std::ptr::null_mut();
    unsafe {
        if ConvertStringSidToSidW(wide.as_ptr(), &mut psid) != 0 && !psid.is_null() {
            Some(AutoSid(psid))
        } else {
            None
        }
    }
}

/// Step 1: Enable required privileges in process token
fn enable_security_privileges() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return false;
        }

        let priv_names = [
            "SeTakeOwnershipPrivilege\0",
            "SeRestorePrivilege\0",
            "SeBackupPrivilege\0",
            "SeSecurityPrivilege\0",
        ];

        for name in priv_names {
            let mut luid: LUID = std::mem::zeroed();
            let name_wide: Vec<u16> = name.encode_utf16().collect();
            if LookupPrivilegeValueW(std::ptr::null(), name_wide.as_ptr(), &mut luid) != 0 {
                let mut tp: TOKEN_PRIVILEGES = std::mem::zeroed();
                tp.PrivilegeCount = 1;
                tp.Privileges[0].Luid = luid;
                tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;

                AdjustTokenPrivileges(
                    token,
                    0,
                    &tp,
                    std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
            }
        }

        CloseHandle(token);
        true
    }
}

/// Step 2: Clear Read-Only attribute and grant FullControl to BUILTIN\Administrators
pub fn take_ownership_and_grant_admin(path: &Path) -> Result<(), io::Error> {
    enable_security_privileges();

    let path_str = path.to_string_lossy().to_string();
    let path_wide = encode_wide(&path_str);

    // 1. Remove FILE_ATTRIBUTE_READONLY if set
    unsafe {
        let attrs = GetFileAttributesW(path_wide.as_ptr());
        if attrs != INVALID_FILE_ATTRIBUTES && (attrs & FILE_ATTRIBUTE_READONLY != 0) {
            SetFileAttributesW(path_wide.as_ptr(), attrs & !FILE_ATTRIBUTE_READONLY);
        }
    }

    // 2. Set Owner to BUILTIN\Administrators via SetNamedSecurityInfoW
    let admin_sid = get_sid(SID_ADMINISTRATORS).ok_or_else(|| {
        io::Error::other("Failed to resolve Administrators SID")
    })?;

    let owner_res = unsafe {
        SetNamedSecurityInfoW(
            path_wide.as_ptr() as *mut u16,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            admin_sid.0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };

    // 3. Grant GENERIC_ALL to Administrators in DACL
    let mut p_sec_desc: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    let mut p_old_dacl: *mut ACL = std::ptr::null_mut();

    let dacl_query_res = unsafe {
        GetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut p_old_dacl,
            std::ptr::null_mut(),
            &mut p_sec_desc,
        )
    };

    let mut new_dacl: *mut ACL = std::ptr::null_mut();
    let mut ea: EXPLICIT_ACCESS_W = unsafe { std::mem::zeroed() };
    ea.grfAccessPermissions = GENERIC_ALL;
    ea.grfAccessMode = GRANT_ACCESS;
    ea.grfInheritance = SUB_CONTAINERS_AND_OBJECTS_INHERIT;
    ea.Trustee.pMultipleTrustee = std::ptr::null_mut();
    ea.Trustee.MultipleTrusteeOperation = NO_MULTIPLE_TRUSTEE;
    ea.Trustee.TrusteeForm = TRUSTEE_IS_SID;
    ea.Trustee.TrusteeType = TRUSTEE_IS_WELL_KNOWN_GROUP;
    ea.Trustee.ptstrName = admin_sid.0 as *mut u16;

    let existing_dacl = if dacl_query_res == ERROR_SUCCESS {
        p_old_dacl
    } else {
        std::ptr::null_mut()
    };

    let acl_res = unsafe { SetEntriesInAclW(1, &ea, existing_dacl, &mut new_dacl) };

    if acl_res == ERROR_SUCCESS && !new_dacl.is_null() {
        unsafe {
            SetNamedSecurityInfoW(
                path_wide.as_ptr() as *mut u16,
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl,
                std::ptr::null_mut(),
            );
            LocalFree(new_dacl as *mut _);
        }
    }

    if !p_sec_desc.is_null() {
        unsafe {
            LocalFree(p_sec_desc as *mut _);
        }
    }

    // 4. If native API hit any snag or if extra insurance is needed, run icacls fallback with exact SID
    if owner_res != ERROR_SUCCESS {
        let _ = Command::new("takeown.exe")
            .args(["/F", &path_str, "/A"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let _ = Command::new("icacls.exe")
            .args([&path_str, "/grant", "*S-1-5-32-544:F", "/C", "/Q"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    Ok(())
}

/// Recursively grant write permissions to directory and all files inside
pub fn take_ownership_and_grant_admin_recursive(dir: &Path) -> Result<(), io::Error> {
    take_ownership_and_grant_admin(dir)?;

    if dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let _ = take_ownership_and_grant_admin(&p);
            }
        }

    Ok(())
}

/// Step 3: Restore ownership back to NT SERVICE\TrustedInstaller and reset DACL
pub fn restore_trusted_installer(path: &Path) -> Result<(), io::Error> {
    enable_security_privileges();

    let path_str = path.to_string_lossy().to_string();
    let path_wide = encode_wide(&path_str);

    let ti_sid = get_sid(SID_TRUSTED_INSTALLER).ok_or_else(|| {
        io::Error::other("Failed to resolve TrustedInstaller SID")
    })?;

    // 1. Native API: Set owner back to TrustedInstaller and reset DACL inheritance
    let owner_res = unsafe {
        SetNamedSecurityInfoW(
            path_wide.as_ptr() as *mut u16,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
            ti_sid.0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };

    // 2. Extra insurance: icacls reset and setowner with SID string (language-independent)
    let _ = Command::new("icacls.exe")
        .args([&path_str, "/reset", "/C", "/Q"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let _ = Command::new("icacls.exe")
        .args([
            &path_str,
            "/setowner",
            &format!("*{}", SID_TRUSTED_INSTALLER),
            "/C",
            "/Q",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    // If SID form is rejected by some older Windows builds, try standard service name as fallback
    let _ = Command::new("icacls.exe")
        .args([
            &path_str,
            "/setowner",
            "NT SERVICE\\TrustedInstaller",
            "/C",
            "/Q",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if owner_res != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(owner_res as i32));
    }

    Ok(())
}

/// Recursively restore TrustedInstaller ownership to directory and all files inside
pub fn restore_trusted_installer_recursive(dir: &Path) {
    if dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let _ = restore_trusted_installer(&p);
            }
        }
    let _ = restore_trusted_installer(dir);
}
