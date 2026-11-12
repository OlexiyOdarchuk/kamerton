// Іконка і опис для .exe на Windows (видно в Провіднику і на панелі задач).
fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // Адреса для звітів вбудовується через option_env!, тож при її зміні треба перезібрати.
    println!("cargo:rerun-if-env-changed=KAMERTON_REPORT_URL");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("FileDescription", "Камертон: завантаження відео і музики")
            .set("ProductName", "Камертон");
        if let Err(e) = res.compile() {
            println!("cargo:warning=не вдалося додати іконку: {e}");
        }
    }
}
