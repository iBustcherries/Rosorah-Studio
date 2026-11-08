//! Prints an activation token from the current Wayland session. browser/test_session.py uses it to
//! stand in for Rosorah when it opens a helper's window.
fn main() {
    match rosorah_activation::standalone_token() {
        Some(token) => println!("{token}"),
        None => {
            eprintln!(
                "No activation token: not a Wayland session, or the compositor lacks xdg-activation"
            );
            std::process::exit(1);
        }
    }
}
