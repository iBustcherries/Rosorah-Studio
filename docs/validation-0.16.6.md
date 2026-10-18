# Rosorah Studio 0.16.6 — account wallet crash hotfix

## Fault and correction

The reported 0.16.5 crash is SIGSEGV in GLib's string interning table during `secret_schema_new` / `secret_schema_attribute_type_get_type`. The wallet wrapper owned a libloading Library handle per operation. After freeing its schema, the wrapper dropped the handle and could unload libsecret even though GLib still retained static type names, enum data and function pointers from that library. A subsequent wallet operation or GLib lookup could dereference unloaded memory.

0.16.6 loads libsecret once through a process-lifetime OnceLock. Individual wallet schemas are still released normally; the library remains resident until process exit. Initialization is shared safely between Twitch and YouTube workers. A missing wallet library still reports a normal error and permits session-only sign-in. No token storage format, schema name, attribute, account marker, OAuth scope, or credential migration changed.

## Verification

- Added a child-process regression that calls the real libsecret and GObject libraries. It constructs/releases a wallet schema and queries the registered GType after releasing the wrapper. Against the unmodified 0.16.5 implementation, the child exited with signal 11 (SIGSEGV). Core dumps were disabled for this intentional negative control.
- With the fix, the same test completes 100 sequential cycles and 800 cycles across eight concurrent threads. No wallet daemon, credentials, or network account is used by the test.
- Release workspace suite: **95 passed, 0 failed, 9 environment-dependent tests ignored**.
- Strict release workspace/all-target Clippy passes. Optimized Linux binary builds successfully and reports version 0.16.6.
- Source comparison limits changes to the wallet implementation/regression test, version metadata and release documentation. Browser, renderer hooks, Steam launch helper, camera, audio, canvas and streaming implementations are unchanged.
- Binary ZIP, tar.gz and source ZIP are verified member-by-member for contents and file modes. Existing companion/native helper files match the 0.16.5 package byte-for-byte.

## Limits

The fault was reproduced locally using real GLib/libsecret libraries; the exact Fedora library build and a live Fedora KWallet session were not available. No real Twitch/YouTube login, token store/restore, chat send or stream was performed here. Existing provider registration requirements and the 0.16.5 account/chat verification limits still apply. No system package, kernel or graphics-driver changes are part of this hotfix.

## Upgrade

Close Rosorah, extract the complete 0.16.6 folder and run `./run-studio.sh`. Existing projects, destinations, remembered-account markers, and wallet credentials remain in their existing locations. No account reset is required by this change; platform-expired/revoked authorization can still require normal reconnecting.
