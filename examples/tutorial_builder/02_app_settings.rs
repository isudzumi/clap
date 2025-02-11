use clap::{app_from_crate, arg, AppSettings};

fn main() {
    let matches = app_from_crate!()
        .args_override_self(true)
        .global_setting(AppSettings::DeriveDisplayOrder)
        .allow_negative_numbers(true)
        .arg(arg!(--two <VALUE>))
        .arg(arg!(--one <VALUE>))
        .get_matches();

    println!("two: {:?}", matches.value_of("two").expect("required"));
    println!("one: {:?}", matches.value_of("one").expect("required"));
}
