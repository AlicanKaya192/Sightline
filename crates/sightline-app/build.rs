fn main() {
    let config = slint_build::CompilerConfiguration::new().with_bundled_translations("lang");
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("failed to compile ui/app-window.slint");
}
