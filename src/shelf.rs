//! File shelf plumbing: files dragged onto the notch arrive through an OLE drop
//! target, and a shelf file is dragged back out with a shell data object.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::*;

/// The drag in progress carries files, so the notch accepts it.
static ACCEPT: AtomicBool = AtomicBool::new(false);

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// Full paths of the files inside a drop.
fn paths_of(data: &IDataObject) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(mut med) = data.GetData(&hdrop_format()) else { return out };
        let hdrop = HDROP(med.u.hGlobal.0);
        let n = DragQueryFileW(hdrop, u32::MAX, None);
        for i in 0..n {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(hdrop, i, Some(&mut buf));
            out.push(String::from_utf16_lossy(&buf[..len]));
        }
        ReleaseStgMedium(&mut med);
    }
    out
}

#[implement(IDropTarget)]
struct Target;

#[allow(non_snake_case)]
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, pdataobj: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let files = pdataobj.as_ref().is_some_and(|d| unsafe { d.QueryGetData(&hdrop_format()) } == S_OK);
        // our own drag-out passing over the notch is not a new drop
        let ours = crate::app().is_some_and(|a| a.shelf_dragging());
        let ok = files && !ours;
        ACCEPT.store(ok, Ordering::Relaxed);
        unsafe { *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        if ok {
            if let Some(a) = crate::app() {
                a.on_shelf_enter();
            }
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = if ACCEPT.load(Ordering::Relaxed) { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        if ACCEPT.swap(false, Ordering::Relaxed) {
            if let Some(a) = crate::app() {
                a.on_shelf_leave();
            }
        }
        Ok(())
    }

    fn Drop(&self, pdataobj: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let accepted = ACCEPT.swap(false, Ordering::Relaxed);
        let paths = pdataobj.as_ref().map(paths_of).unwrap_or_default();
        unsafe { *effect = if accepted && !paths.is_empty() { DROPEFFECT_COPY } else { DROPEFFECT_NONE } };
        if let Some(a) = crate::app() {
            a.on_shelf_drop(paths);
        }
        Ok(())
    }
}

/// Let the notch window receive dropped files. Needs OleInitialize on this thread.
pub fn register(hwnd: HWND) {
    let target: IDropTarget = Target.into();
    unsafe {
        let _ = RegisterDragDrop(hwnd, &target);
    }
}

/// Start dragging `path` out of the notch. Blocks until the drop or cancel.
pub fn drag_out(hwnd: HWND, path: &str) {
    unsafe {
        let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path), None::<&IBindCtx>) else { return };
        let Ok(data) = item.BindToHandler::<_, IDataObject>(None::<&IBindCtx>, &BHID_DataObject) else { return };
        let _ = SHDoDragDrop(Some(hwnd), &data, None::<&IDropSource>, DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK);
    }
}
