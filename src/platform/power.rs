use windows::Win32::UI::WindowsAndMessaging::PBT_APMRESUMEAUTOMATIC;

pub fn is_resume_event(wparam: usize) -> bool {
    wparam as u32 == PBT_APMRESUMEAUTOMATIC
}
