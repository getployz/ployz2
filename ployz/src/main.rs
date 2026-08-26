use std::io::ErrorKind;
use std::panic::PanicHookInfo;
use std::process::{ExitCode, exit};

fn main() -> ExitCode {
    exit_quietly_on_broken_pipe();
    ployz::failure::terminate(ployz::handlers::run())
}

fn exit_quietly_on_broken_pipe() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if is_broken_pipe(info) {
            exit(141);
        }
        default_hook(info);
    }));
}

fn is_broken_pipe(info: &PanicHookInfo<'_>) -> bool {
    if let Some(error) = info.payload().downcast_ref::<std::io::Error>() {
        return error.kind() == ErrorKind::BrokenPipe;
    }
    payload_text(info).is_some_and(|text| text.contains("Broken pipe"))
}

fn payload_text<'a>(info: &'a PanicHookInfo<'_>) -> Option<&'a str> {
    info.payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str))
}
