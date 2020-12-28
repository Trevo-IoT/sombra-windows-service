use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};
use std::ffi::OsString;
use rand::Rng;
use std::time::Duration;
use state::LocalStorage;
use windows_service::service_control_handler::ServiceStatusHandle;
use std::sync::mpsc::Receiver;
use std::io::Write;

define_windows_service!(ffi_service, service_entry);

static SERVICE_NAME: state::LocalStorage<String> = LocalStorage::new();

const STOP_SERVICE_CODE: u32 = 1;
const CHILD_PROCESS_ERROR_CODE: u32 = 2;
const UNKNOWN_ERROR_CODE: i32 = 3;
const TARGET_PROCESS_NOT_FOUND_CODE: u32 = 4;
const ARGUMENT_DECODE_ERROR_CODE: u32 = 5;
const ARGUMENT_AMOUNT_ERROR_CODE: u32 = 6;

fn service_entry(arguments: Vec<OsString>) {
    if let Err(_e) = service_exec(arguments) {
        // TODO: Logging error
    }
}

fn service_exec(arguments: Vec<OsString>) -> windows_service::Result<()> {
    // Stop event handler
    let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop => {
                shutdown_tx.send(()).unwrap();
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };
    let status_handle = service_control_handler::register(
        SERVICE_NAME.get(), event_handler)?;
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    // Target process handle and udp server
    let exit_code = service_loop(arguments, shutdown_rx);

    // Tell the system that service has stopped
    status_handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(exit_code),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })
}

fn service_loop(arguments: Vec<OsString>, shutdown_rx: Receiver<()>) -> u32 {
    if arguments.len() < 2 {
        return ARGUMENT_AMOUNT_ERROR_CODE;
    }

    let path = if let Some(arg1) = arguments[1].to_str() {
        if let Ok(path) = dunce::canonicalize(arg1) {
            path
        } else {
            return TARGET_PROCESS_NOT_FOUND_CODE;
        }
    } else {
        return ARGUMENT_DECODE_ERROR_CODE;
    };

    let mut exit_code = 0;
    if let Ok(mut child) = std::process::Command::new(path).spawn() {
        'process_loop: loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exit_code = status.code().unwrap_or(UNKNOWN_ERROR_CODE) as u32;
                    break 'process_loop;
                },
                Ok(None) => {
                    if shutdown_rx.try_recv().is_ok() {
                        exit_code = STOP_SERVICE_CODE;
                        child.kill();
                        break 'process_loop;
                    }
                },
                Err(_e) => {
                    exit_code = CHILD_PROCESS_ERROR_CODE;
                }
            }
        }
    } else {
        // TODO: Logging error
        exit_code = CHILD_PROCESS_ERROR_CODE;
    }

    exit_code
}

fn main() -> std::io::Result<()> {
    let service_name = format!("sombra-windows-service-{}", rand::thread_rng().gen::<u64>());
    println!("{}", service_name);
    SERVICE_NAME.set(move || service_name.clone());

    service_dispatcher::start(SERVICE_NAME.get(), ffi_service);

    Ok(())
}
