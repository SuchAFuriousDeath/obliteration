use cbindgen::{Builder, Config, Language, Style};
use slint_build::CompilerConfiguration;
use std::path::PathBuf;

const LINUX_INCLUDE: &str = r#"
#ifdef __linux__
#include <linux/kvm.h>
#endif
"#;

fn main() {
    if std::env::var("CARGO_FEATURE_SLINT").is_ok_and(|var| var == "1") {
        build_bin();
    }

    if std::env::var("CARGO_FEATURE_QT").is_ok_and(|var| var == "1") {
        build_lib();
    }
}

fn build_bin() {
    // Compile main
    slint_build::compile_with_config(
        "slint/main.slint",
        CompilerConfiguration::new().with_style(String::from("fluent-dark")),
    )
    .unwrap();
}

fn build_lib() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let mut conf = Config::default();
    let mut buf = String::new();

    buf.push_str(LINUX_INCLUDE);

    for ext in ["Param", "Pkg"] {
        buf.push_str("\nstruct ");
        buf.push_str(ext);
        buf.push(';');
    }

    conf.after_includes = Some(buf);
    conf.pragma_once = true;
    conf.tab_width = 4;
    conf.language = Language::C;
    conf.cpp_compat = true;
    conf.style = Style::Tag;
    conf.usize_is_size_t = true;
    conf.export.exclude.push("KvmRegs".into());
    conf.export.exclude.push("KvmSpecialRegs".into());
    conf.export.exclude.push("KvmTranslation".into());
    conf.export
        .rename
        .insert("KvmRegs".into(), "kvm_regs".into());
    conf.export
        .rename
        .insert("KvmSpecialRegs".into(), "kvm_sregs".into());
    conf.export
        .rename
        .insert("KvmTranslation".into(), "kvm_translation".into());
    conf.enumeration.prefix_with_name = true;
    conf.defines
        .insert("target_os = linux".into(), "__linux__".into());
    conf.defines
        .insert("target_os = macos".into(), "__APPLE__".into());

    Builder::new()
        .with_crate(&root)
        .with_config(conf)
        .generate()
        .unwrap()
        .write_to_file(root.join("core.h"));
}
