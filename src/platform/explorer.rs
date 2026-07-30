use anyhow::{bail, Result};
#[cfg(windows)]
use anyhow::Context;
use std::path::PathBuf;

#[cfg(windows)]
pub struct ExplorerTracker {
    thread_id: u32,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl Drop for ExplorerTracker {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

        unsafe {
            PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(windows)]
pub fn start_explorer_tracker<F>(on_folder_visited: F) -> Result<ExplorerTracker>
where
    F: Fn(PathBuf) + Send + 'static,
{
    use std::{sync::mpsc, time::Duration};

    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let callback = Box::new(on_folder_visited);
    let handle = std::thread::Builder::new()
        .name("fastaccess-explorer-events".into())
        .spawn(move || unsafe {
            run_tracker(callback, ready_tx);
        })
        .context("cannot start Explorer event thread")?;

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(thread_id)) => Ok(ExplorerTracker {
            thread_id,
            handle: Some(handle),
        }),
        Ok(Err(message)) => {
            let _ = handle.join();
            bail!("{message}")
        }
        Err(error) => {
            bail!("Explorer event thread did not initialize: {error}")
        }
    }
}

#[cfg(not(windows))]
pub struct ExplorerTracker;

#[cfg(not(windows))]
pub fn start_explorer_tracker<F>(_on_folder_visited: F) -> Result<ExplorerTracker>
where
    F: Fn(PathBuf) + Send + 'static,
{
    bail!("Explorer navigation tracking is supported only on Windows")
}

#[cfg(windows)]
mod win32 {
    // Explorer exposes navigation through COM connection-point events. Keeping
    // the subscriptions on one STA thread gives us push-based updates with no
    // polling timer and no filesystem reads on the navigation path.
    use std::{
        ffi::{c_void, OsString},
        mem::zeroed,
        os::windows::ffi::OsStringExt,
        path::PathBuf,
        ptr,
        sync::atomic::{AtomicU32, Ordering},
    };
    use windows_sys::{
        core::GUID,
        Win32::{
            Foundation::SysStringLen,
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER,
                    COINIT_APARTMENTTHREADED, DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPPARAMS,
                },
                Threading::GetCurrentThreadId,
                Variant::{VariantClear, VARIANT, VT_BSTR, VT_DISPATCH, VT_I4},
            },
            UI::{
                Shell::{PathCreateFromUrlW, ShellWindows},
                WindowsAndMessaging::{
                    DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW,
                    TranslateMessage, MSG, PM_NOREMOVE, WM_APP,
                },
            },
        },
    };

    const WM_REFRESH_SUBSCRIPTIONS: u32 = WM_APP + 0x41;

    const S_OK: i32 = 0;
    const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
    const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
    const E_POINTER: i32 = 0x8000_4003u32 as i32;
    const DISPID_NAVIGATECOMPLETE2: i32 = 252;
    const DISPID_WINDOWREGISTERED: i32 = 200;
    const DISPID_WINDOWREVOKED: i32 = 201;

    const IID_NULL: GUID = GUID::from_u128(0);
    const IID_IUNKNOWN: GUID =
        GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
    const IID_IDISPATCH: GUID =
        GUID::from_u128(0x00020400_0000_0000_c000_000000000046);
    const IID_ICONNECTIONPOINTCONTAINER: GUID =
        GUID::from_u128(0xb196b284_bab4_101a_b69c_00aa00341d07);
    const DIID_DWEBBROWSEREVENTS2: GUID =
        GUID::from_u128(0x34a715a0_6587_11d0_924a_0020afc7ac4d);
    const DIID_DSHELLWINDOWSEVENTS: GUID =
        GUID::from_u128(0xfe4106e0_399a_11d0_a48c_00a0c90a8f39);

    #[repr(C)]
    struct IDispatchRaw {
        vtable: *const IDispatchVTable,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct IDispatchVTable {
        query_interface: unsafe extern "system" fn(
            *mut IDispatchRaw,
            *const GUID,
            *mut *mut c_void,
        ) -> i32,
        add_ref: unsafe extern "system" fn(*mut IDispatchRaw) -> u32,
        release: unsafe extern "system" fn(*mut IDispatchRaw) -> u32,
        get_type_info_count:
            unsafe extern "system" fn(*mut IDispatchRaw, *mut u32) -> i32,
        get_type_info: unsafe extern "system" fn(
            *mut IDispatchRaw,
            u32,
            u32,
            *mut *mut c_void,
        ) -> i32,
        get_ids_of_names: unsafe extern "system" fn(
            *mut IDispatchRaw,
            *const GUID,
            *mut *mut u16,
            u32,
            u32,
            *mut i32,
        ) -> i32,
        invoke: unsafe extern "system" fn(
            *mut IDispatchRaw,
            i32,
            *const GUID,
            u32,
            u16,
            *mut DISPPARAMS,
            *mut VARIANT,
            *mut c_void,
            *mut u32,
        ) -> i32,
    }

    #[repr(C)]
    struct IConnectionPointContainerRaw {
        vtable: *const IConnectionPointContainerVTable,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct IConnectionPointContainerVTable {
        query_interface: unsafe extern "system" fn(
            *mut IConnectionPointContainerRaw,
            *const GUID,
            *mut *mut c_void,
        ) -> i32,
        add_ref:
            unsafe extern "system" fn(*mut IConnectionPointContainerRaw) -> u32,
        release:
            unsafe extern "system" fn(*mut IConnectionPointContainerRaw) -> u32,
        enum_connection_points: unsafe extern "system" fn(
            *mut IConnectionPointContainerRaw,
            *mut *mut c_void,
        ) -> i32,
        find_connection_point: unsafe extern "system" fn(
            *mut IConnectionPointContainerRaw,
            *const GUID,
            *mut *mut IConnectionPointRaw,
        ) -> i32,
    }

    #[repr(C)]
    struct IConnectionPointRaw {
        vtable: *const IConnectionPointVTable,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct IConnectionPointVTable {
        query_interface: unsafe extern "system" fn(
            *mut IConnectionPointRaw,
            *const GUID,
            *mut *mut c_void,
        ) -> i32,
        add_ref: unsafe extern "system" fn(*mut IConnectionPointRaw) -> u32,
        release: unsafe extern "system" fn(*mut IConnectionPointRaw) -> u32,
        get_connection_interface:
            unsafe extern "system" fn(*mut IConnectionPointRaw, *mut GUID) -> i32,
        get_connection_point_container: unsafe extern "system" fn(
            *mut IConnectionPointRaw,
            *mut *mut IConnectionPointContainerRaw,
        ) -> i32,
        advise: unsafe extern "system" fn(
            *mut IConnectionPointRaw,
            *mut c_void,
            *mut u32,
        ) -> i32,
        unadvise:
            unsafe extern "system" fn(*mut IConnectionPointRaw, u32) -> i32,
        enum_connections:
            unsafe extern "system" fn(*mut IConnectionPointRaw, *mut *mut c_void) -> i32,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct EventSink {
        vtable: *const IDispatchVTable,
        references: AtomicU32,
        owner_thread_id: u32,
        callback: Box<dyn Fn(PathBuf) + Send>,
    }

    struct Subscription {
        connection_point: *mut IConnectionPointRaw,
        cookie: u32,
    }

    struct BrowserSubscription {
        identity: usize,
        subscription: Subscription,
    }

    pub(super) unsafe fn run_tracker(
        callback: Box<dyn Fn(PathBuf) + Send>,
        ready: std::sync::mpsc::SyncSender<Result<u32, String>>,
    ) {
        let init_result = CoInitializeEx(
            ptr::null(),
            COINIT_APARTMENTTHREADED as u32,
        );
        if init_result < 0 {
            let _ = ready.send(Err(format!(
                "CoInitializeEx failed: HRESULT 0x{:08X}",
                init_result as u32
            )));
            return;
        }

        let thread_id = GetCurrentThreadId();
        let mut queue_message: MSG = zeroed();
        PeekMessageW(
            &mut queue_message,
            ptr::null_mut(),
            0,
            0,
            PM_NOREMOVE,
        );

        let shell_windows = match create_shell_windows() {
            Ok(dispatch) => dispatch,
            Err(message) => {
                let _ = ready.send(Err(message));
                CoUninitialize();
                return;
            }
        };

        let sink = create_event_sink(thread_id, callback);
        let shell_subscription =
            match subscribe(shell_windows, &DIID_DSHELLWINDOWSEVENTS, sink) {
                Ok(subscription) => subscription,
                Err(message) => {
                    let _ = ready.send(Err(message));
                    dispatch_release(sink);
                    dispatch_release(shell_windows);
                    CoUninitialize();
                    return;
                }
            };
        let mut browser_subscriptions = Vec::new();
        refresh_browser_subscriptions(
            shell_windows,
            sink,
            &mut browser_subscriptions,
            false,
        );

        if ready.send(Ok(thread_id)).is_err() {
            cleanup_subscription(shell_subscription);
            cleanup_browser_subscriptions(&mut browser_subscriptions);
            dispatch_release(sink);
            dispatch_release(shell_windows);
            CoUninitialize();
            return;
        }

        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            if message.message == WM_REFRESH_SUBSCRIPTIONS {
                refresh_browser_subscriptions(
                    shell_windows,
                    sink,
                    &mut browser_subscriptions,
                    true,
                );
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        cleanup_browser_subscriptions(&mut browser_subscriptions);
        cleanup_subscription(shell_subscription);
        dispatch_release(sink);
        dispatch_release(shell_windows);
        CoUninitialize();
    }

    unsafe fn create_shell_windows() -> Result<*mut IDispatchRaw, String> {
        let mut dispatch = ptr::null_mut();
        let result = CoCreateInstance(
            &ShellWindows,
            ptr::null_mut(),
            CLSCTX_LOCAL_SERVER,
            &IID_IDISPATCH,
            &mut dispatch,
        );
        if result < 0 || dispatch.is_null() {
            return Err(format!(
                "cannot create ShellWindows: HRESULT 0x{:08X}",
                result as u32
            ));
        }
        Ok(dispatch.cast())
    }

    unsafe fn create_event_sink(
        owner_thread_id: u32,
        callback: Box<dyn Fn(PathBuf) + Send>,
    ) -> *mut IDispatchRaw {
        Box::into_raw(Box::new(EventSink {
            vtable: &EVENT_SINK_VTABLE,
            references: AtomicU32::new(1),
            owner_thread_id,
            callback,
        }))
        .cast()
    }

    unsafe fn subscribe(
        source: *mut IDispatchRaw,
        event_iid: &GUID,
        sink: *mut IDispatchRaw,
    ) -> Result<Subscription, String> {
        let mut container = ptr::null_mut();
        let query_result = ((*(*source).vtable).query_interface)(
            source,
            &IID_ICONNECTIONPOINTCONTAINER,
            &mut container,
        );
        if query_result < 0 || container.is_null() {
            return Err(format!(
                "source has no connection-point container: HRESULT 0x{:08X}",
                query_result as u32
            ));
        }
        let container = container.cast::<IConnectionPointContainerRaw>();

        let mut connection_point = ptr::null_mut();
        let find_result = ((*(*container).vtable).find_connection_point)(
            container,
            event_iid,
            &mut connection_point,
        );
        ((*(*container).vtable).release)(container);
        if find_result < 0 || connection_point.is_null() {
            return Err(format!(
                "cannot find Explorer event connection point: HRESULT 0x{:08X}",
                find_result as u32
            ));
        }

        let mut cookie = 0;
        let advise_result = ((*(*connection_point).vtable).advise)(
            connection_point,
            sink.cast(),
            &mut cookie,
        );
        if advise_result < 0 {
            ((*(*connection_point).vtable).release)(connection_point);
            return Err(format!(
                "cannot subscribe to Explorer events: HRESULT 0x{:08X}",
                advise_result as u32
            ));
        }

        Ok(Subscription {
            connection_point,
            cookie,
        })
    }

    unsafe fn refresh_browser_subscriptions(
        shell_windows: *mut IDispatchRaw,
        sink: *mut IDispatchRaw,
        subscriptions: &mut Vec<BrowserSubscription>,
        record_new_current_location: bool,
    ) {
        let Some(count) = dispatch_i32_property(shell_windows, "Count") else {
            return;
        };

        let mut current_windows = Vec::new();
        for index in 0..count.max(0) {
            let Some(dispatch) = dispatch_item(shell_windows, index) else {
                continue;
            };
            if !is_explorer_window(dispatch) {
                dispatch_release(dispatch);
            } else if let Some(identity) = dispatch_identity(dispatch) {
                current_windows.push((identity, dispatch));
            } else {
                dispatch_release(dispatch);
            }
        }

        subscriptions.retain(|subscription| {
            let still_open = current_windows
                .iter()
                .any(|(identity, _)| *identity == subscription.identity);
            if !still_open {
                cleanup_subscription_fields(&subscription.subscription);
            }
            still_open
        });

        for (identity, dispatch) in current_windows {
            if !subscriptions
                .iter()
                .any(|subscription| subscription.identity == identity)
            {
                if let Ok(subscription) =
                    subscribe(dispatch, &DIID_DWEBBROWSEREVENTS2, sink)
                {
                    subscriptions.push(BrowserSubscription {
                        identity,
                        subscription,
                    });
                    if record_new_current_location {
                        notify_dispatch_location(sink, dispatch);
                    }
                }
            }
            dispatch_release(dispatch);
        }
    }

    unsafe fn cleanup_browser_subscriptions(
        subscriptions: &mut Vec<BrowserSubscription>,
    ) {
        for subscription in subscriptions.drain(..) {
            cleanup_subscription(subscription.subscription);
        }
    }

    unsafe fn cleanup_subscription(subscription: Subscription) {
        cleanup_subscription_fields(&subscription);
    }

    unsafe fn cleanup_subscription_fields(subscription: &Subscription) {
        ((*(*subscription.connection_point).vtable).unadvise)(
            subscription.connection_point,
            subscription.cookie,
        );
        ((*(*subscription.connection_point).vtable).release)(
            subscription.connection_point,
        );
    }

    unsafe fn dispatch_identity(dispatch: *mut IDispatchRaw) -> Option<usize> {
        let mut identity = ptr::null_mut();
        let result = ((*(*dispatch).vtable).query_interface)(
            dispatch,
            &IID_IUNKNOWN,
            &mut identity,
        );
        if result < 0 || identity.is_null() {
            return None;
        }
        let key = identity as usize;
        dispatch_release(identity.cast());
        Some(key)
    }

    unsafe fn is_explorer_window(dispatch: *mut IDispatchRaw) -> bool {
        dispatch_string_property(dispatch, "FullName")
            .and_then(|path| {
                std::path::Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .is_some_and(|name| name.eq_ignore_ascii_case("explorer.exe"))
    }

    unsafe fn dispatch_i32_property(
        dispatch: *mut IDispatchRaw,
        name: &str,
    ) -> Option<i32> {
        let mut result = dispatch_invoke(dispatch, name, DISPATCH_PROPERTYGET, &mut [])?;
        let value = if variant_type(&result) == VT_I4 {
            Some(result.Anonymous.Anonymous.Anonymous.lVal)
        } else {
            None
        };
        VariantClear(&mut result);
        value
    }

    unsafe fn dispatch_string_property(
        dispatch: *mut IDispatchRaw,
        name: &str,
    ) -> Option<String> {
        let mut result = dispatch_invoke(dispatch, name, DISPATCH_PROPERTYGET, &mut [])?;
        let value = if variant_type(&result) == VT_BSTR {
            let bstr = result.Anonymous.Anonymous.Anonymous.bstrVal;
            if bstr.is_null() {
                None
            } else {
                let length = SysStringLen(bstr) as usize;
                Some(String::from_utf16_lossy(std::slice::from_raw_parts(
                    bstr, length,
                )))
            }
        } else {
            None
        };
        VariantClear(&mut result);
        value
    }

    unsafe fn dispatch_item(
        dispatch: *mut IDispatchRaw,
        index: i32,
    ) -> Option<*mut IDispatchRaw> {
        let mut argument: VARIANT = zeroed();
        argument.Anonymous.Anonymous.vt = VT_I4;
        argument.Anonymous.Anonymous.Anonymous.lVal = index;

        let mut result = dispatch_invoke(
            dispatch,
            "Item",
            DISPATCH_METHOD | DISPATCH_PROPERTYGET,
            std::slice::from_mut(&mut argument),
        )?;
        let value = if variant_type(&result) == VT_DISPATCH {
            let value = result.Anonymous.Anonymous.Anonymous.pdispVal
                as *mut IDispatchRaw;
            if value.is_null() {
                None
            } else {
                dispatch_add_ref(value);
                Some(value)
            }
        } else {
            None
        };
        VariantClear(&mut result);
        value
    }

    unsafe fn dispatch_invoke(
        dispatch: *mut IDispatchRaw,
        name: &str,
        flags: u16,
        arguments: &mut [VARIANT],
    ) -> Option<VARIANT> {
        let dispatch_id = dispatch_id(dispatch, name)?;
        let mut parameters = DISPPARAMS {
            rgvarg: if arguments.is_empty() {
                ptr::null_mut()
            } else {
                arguments.as_mut_ptr()
            },
            rgdispidNamedArgs: ptr::null_mut(),
            cArgs: arguments.len() as u32,
            cNamedArgs: 0,
        };
        let mut result: VARIANT = zeroed();
        let invoke_result = ((*(*dispatch).vtable).invoke)(
            dispatch,
            dispatch_id,
            &IID_NULL,
            0,
            flags,
            &mut parameters,
            &mut result,
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if invoke_result < 0 {
            VariantClear(&mut result);
            return None;
        }
        Some(result)
    }

    unsafe fn dispatch_id(
        dispatch: *mut IDispatchRaw,
        name: &str,
    ) -> Option<i32> {
        let mut wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut name_pointer = wide.as_mut_ptr();
        let mut dispatch_id = 0;
        let result = ((*(*dispatch).vtable).get_ids_of_names)(
            dispatch,
            &IID_NULL,
            &mut name_pointer,
            1,
            0,
            &mut dispatch_id,
        );
        (result >= 0).then_some(dispatch_id)
    }

    unsafe fn dispatch_add_ref(dispatch: *mut IDispatchRaw) {
        ((*(*dispatch).vtable).add_ref)(dispatch);
    }

    unsafe fn dispatch_release(dispatch: *mut IDispatchRaw) {
        if !dispatch.is_null() {
            ((*(*dispatch).vtable).release)(dispatch);
        }
    }

    unsafe fn variant_type(value: &VARIANT) -> u16 {
        value.Anonymous.Anonymous.vt
    }

    unsafe fn path_from_url(url: &str) -> Option<PathBuf> {
        if !url.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("file:")) {
            return None;
        }

        let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let mut buffer = vec![0u16; 32_768];
        let mut length = buffer.len() as u32;
        let result = PathCreateFromUrlW(
            wide.as_ptr(),
            buffer.as_mut_ptr(),
            &mut length,
            0,
        );
        if result < 0 || length == 0 {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(
            &buffer[..length as usize],
        )))
    }

    unsafe extern "system" fn sink_query_interface(
        this: *mut IDispatchRaw,
        iid: *const GUID,
        interface: *mut *mut c_void,
    ) -> i32 {
        if iid.is_null() || interface.is_null() {
            return E_POINTER;
        }
        *interface = ptr::null_mut();
        let iid = &*iid;
        if guid_eq(iid, &IID_IUNKNOWN)
            || guid_eq(iid, &IID_IDISPATCH)
            || guid_eq(iid, &DIID_DWEBBROWSEREVENTS2)
            || guid_eq(iid, &DIID_DSHELLWINDOWSEVENTS)
        {
            *interface = this.cast();
            sink_add_ref(this);
            S_OK
        } else {
            E_NOINTERFACE
        }
    }

    unsafe extern "system" fn sink_add_ref(this: *mut IDispatchRaw) -> u32 {
        let sink = &*(this as *mut EventSink);
        sink.references.fetch_add(1, Ordering::Relaxed) + 1
    }

    unsafe extern "system" fn sink_release(this: *mut IDispatchRaw) -> u32 {
        let sink = &*(this as *mut EventSink);
        let remaining = sink.references.fetch_sub(1, Ordering::AcqRel) - 1;
        if remaining == 0 {
            drop(Box::from_raw(this as *mut EventSink));
        }
        remaining
    }

    unsafe extern "system" fn sink_get_type_info_count(
        _this: *mut IDispatchRaw,
        count: *mut u32,
    ) -> i32 {
        if count.is_null() {
            return E_POINTER;
        }
        *count = 0;
        S_OK
    }

    unsafe extern "system" fn sink_get_type_info(
        _this: *mut IDispatchRaw,
        _info: u32,
        _locale: u32,
        _type_info: *mut *mut c_void,
    ) -> i32 {
        E_NOTIMPL
    }

    unsafe extern "system" fn sink_get_ids_of_names(
        _this: *mut IDispatchRaw,
        _iid: *const GUID,
        _names: *mut *mut u16,
        _count: u32,
        _locale: u32,
        _dispatch_ids: *mut i32,
    ) -> i32 {
        E_NOTIMPL
    }

    unsafe extern "system" fn sink_invoke(
        this: *mut IDispatchRaw,
        dispatch_id: i32,
        _iid: *const GUID,
        _locale: u32,
        _flags: u16,
        parameters: *mut DISPPARAMS,
        _result: *mut VARIANT,
        _exception: *mut c_void,
        _argument_error: *mut u32,
    ) -> i32 {
        let sink = &*(this as *mut EventSink);
        match dispatch_id {
            DISPID_WINDOWREGISTERED | DISPID_WINDOWREVOKED => {
                PostThreadMessageW(
                    sink.owner_thread_id,
                    WM_REFRESH_SUBSCRIPTIONS,
                    0,
                    0,
                );
            }
            DISPID_NAVIGATECOMPLETE2 => {
                if let Some(dispatch) = event_dispatch(parameters) {
                    notify_dispatch_location(this, dispatch);
                }
            }
            _ => {}
        }
        S_OK
    }

    unsafe fn notify_dispatch_location(
        sink_dispatch: *mut IDispatchRaw,
        browser_dispatch: *mut IDispatchRaw,
    ) {
        let sink = &*(sink_dispatch as *mut EventSink);
        if let Some(url) = dispatch_string_property(browser_dispatch, "LocationURL") {
            if let Some(path) = path_from_url(&url) {
                let _ = std::panic::catch_unwind(
                    std::panic::AssertUnwindSafe(|| {
                        (sink.callback.as_ref())(path)
                    }),
                );
            }
        }
    }

    unsafe fn event_dispatch(
        parameters: *mut DISPPARAMS,
    ) -> Option<*mut IDispatchRaw> {
        if parameters.is_null() {
            return None;
        }
        let parameters = &*parameters;
        if parameters.rgvarg.is_null() {
            return None;
        }
        std::slice::from_raw_parts(parameters.rgvarg, parameters.cArgs as usize)
            .iter()
            .find(|argument| variant_type(argument) == VT_DISPATCH)
            .map(|argument| {
                argument.Anonymous.Anonymous.Anonymous.pdispVal
                    as *mut IDispatchRaw
            })
            .filter(|dispatch| !dispatch.is_null())
    }

    fn guid_eq(left: &GUID, right: &GUID) -> bool {
        left.data1 == right.data1
            && left.data2 == right.data2
            && left.data3 == right.data3
            && left.data4 == right.data4
    }

    static EVENT_SINK_VTABLE: IDispatchVTable = IDispatchVTable {
        query_interface: sink_query_interface,
        add_ref: sink_add_ref,
        release: sink_release,
        get_type_info_count: sink_get_type_info_count,
        get_type_info: sink_get_type_info,
        get_ids_of_names: sink_get_ids_of_names,
        invoke: sink_invoke,
    };

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn converts_explorer_file_url_to_windows_path() {
            let path = unsafe { path_from_url("file:///D:/Projects/Fast%20Access") }
                .expect("file URL should convert");

            assert_eq!(path, PathBuf::from(r"D:\Projects\Fast Access"));
        }

        #[test]
        fn ignores_virtual_shell_urls() {
            assert!(unsafe { path_from_url("shell:::{679F85CB-0220-4080-B29B-5540CC05AAB6}") }
                .is_none());
        }
    }
}

#[cfg(windows)]
use win32::run_tracker;
