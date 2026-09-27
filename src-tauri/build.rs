fn main() {
    // The same manifest must reach both the application and lib.rs unit-test
    // executables. Tauri's default resource is linked only into the app bin.
    let attributes = tauri_build::Attributes::new()
        .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attributes).expect("failed to build Tauri resources");

    // Without Common Controls v6, the Windows loader cannot resolve
    // TaskDialogIndirect and exits before the test harness starts.
    embed_resource::compile_for_everything("windows-common-controls.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed the Windows application manifest");
}
