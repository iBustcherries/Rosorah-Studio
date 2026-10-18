//! Beginner setup, deliberately separate from server-verified connection state.
use auroracast_core::donations::{Provider, Setup, Step};
use eframe::egui;

/// Returns (preferences changed, optional simulated preview).
pub fn ui(
    ui: &mut egui::Ui,
    setup: &mut Setup,
    connection: &mut crate::donations_live::Hub,
    kofi: &mut crate::kofi::Hub,
) -> (bool, Option<String>) {
    let before = serde_json::to_string(setup).unwrap_or_default();
    let mut preview = None;
    ui.heading("Donation setup");
    ui.label("Connect a donation service once, then use its events in your alerts and goals.");
    ui.add_space(8.);
    let status = connection.status();
    ui.label(format!("StreamElements / Streamlabs: {}", status.text));
    if (!status.owner.is_empty() || connection.has_saved_connection())
        && ui.small_button("Disconnect donation service").clicked()
    {
        connection.disconnect();
    }
    let kofi_status = kofi.status();
    ui.label(&kofi_status.text);
    if !kofi_status.owner.is_empty() && ui.small_button("Disconnect Ko-fi").clicked() {
        kofi.disconnect();
    }
    ui.add_space(12.);
    ui.horizontal_wrapped(|ui| {
        for (step, label) in [
            (Step::Choose, "1 · Choose"),
            (Step::Prepare, "2 · Prepare"),
            (Step::Connect, "3 · Connect"),
            (Step::Customize, "4 · Customize"),
            (Step::Preview, "5 · Preview"),
        ] {
            if ui.selectable_label(setup.step == step, label).clicked() {
                setup.step = step;
                setup.started = true;
            }
        }
    });
    ui.add_space(16.);
    ui.group(|ui| {
        ui.set_min_width(ui.available_width().min(650.));
        match setup.step {
            Step::Choose => {
                ui.strong("Where do you receive donations?");
                ui.label("Choose the service you already use. Using the same service avoids showing one donation twice.");
                for provider in Provider::ALL { ui.radio_value(&mut setup.provider, provider, provider.name()); }
                ui.add_space(10.);
                ui.label("Already accept PayPal through StreamElements or Streamlabs? Choose that service above. Ko-fi can connect alongside either service through a hosted receiver. A separate PayPal connection is unnecessary for those donations.");
                ui.label("You keep control of which services you connect. This setup does not move money or create a payment.");
            }
            Step::Prepare => {
                ui.strong(format!("Before connecting {}",setup.provider.name()));
                if setup.provider == Provider::PayPal {
                    ui.label("1. Open PayPal using the button below and sign in there.");
                    ui.label("2. Complete any email confirmation, identity checks, or account limitations shown by PayPal.");
                    ui.label("3. For alerts in this build, choose StreamElements or Streamlabs and connect PayPal in that service’s tipping settings. Review the account requirements and permissions on the provider’s website.");
                    ui.label("4. Finish the provider’s donation-page setup, then return here and select that provider. Rosorah needs its event token, never your PayPal password.");
                    ui.hyperlink_to("Open PayPal securely", Provider::PayPal.website());
                    ui.separator();
                    ui.label("Only payments made through the connected donation page produce these alerts. Personal transfers and PayPal.me payments are not automatically imported. A test alert is a simulation; it does not prove that a real payment was received.");
                } else {
                    ui.label(format!("1. Sign in to {} on its official website.",setup.provider.name()));
                    ui.label("2. Complete its donation-page setup and connect your preferred payment method there.");
                    ui.label("3. Keep that page handy, then return to Rosorah for the event connection.");
                    ui.hyperlink_to(format!("Open {}",setup.provider.name()),setup.provider.website());
                }
                ui.label("You can close this page and resume later. Your choices are saved with your project.");
            }
            Step::Connect => {
                match setup.provider {
                    Provider::StreamElements | Provider::Streamlabs => {
                        ui.strong(format!("Connect {} donation events", setup.provider.name()));
                        if setup.provider == Provider::StreamElements {
                            ui.label("1. Open your StreamElements dashboard and select the channel that owns your tipping page.");
                            ui.label("2. In your account/channel settings, copy the channel ID and JWT token. Paste them below.");
                            ui.hyperlink_to("Open StreamElements dashboard", "https://streamelements.com/dashboard");
                            ui.label("Channel ID");
                            ui.add(egui::TextEdit::singleline(&mut connection.room).char_limit(24).hint_text("24-character channel ID"));
                            ui.label("JWT token");
                        } else {
                            ui.label("1. Open Streamlabs account settings > API Settings > API Tokens.");
                            ui.label("2. Copy the Socket API Token and paste it below. Use the socket token, not a stream key or widget URL.");
                            ui.hyperlink_to("Open Streamlabs API settings", "https://streamlabs.com/dashboard#/settings/api-settings");
                            ui.label("Socket API Token");
                        }
                        ui.add(egui::TextEdit::singleline(&mut connection.token).password(true).char_limit(8192).hint_text("Paste your event token here").background_color(egui::Color32::from_rgb(32,32,39)).desired_width(f32::INFINITY));
                        ui.checkbox(&mut connection.remember,"Remember on this computer (desktop wallet)");
                        ui.small("Your token stays outside scene projects and logs. Rosorah reconnects automatically while running; remembering restores the connection next time.");
                        if ui.add_enabled(!connection.token.trim().is_empty(), egui::Button::new("Connect donation events")).clicked() { connection.connect(setup.provider); }
                        ui.label("3. Wait for Connected above. Then choose a Donation alert rule or add a Donation goal widget. Only matching currencies increase a goal.");
                        ui.small("StreamElements or Streamlabs can run alongside Ko-fi. Avoid forwarding the same payment through multiple services. Rosorah receives new events while open; it does not import an account balance or missed payments. Refunds and chargebacks are not automatically subtracted.");
                    }
                    Provider::Kofi => {
                        ui.strong("Connect Ko-fi");
                        ui.label("1. Open Ko-fi webhook settings and copy your verification token below.");
                        ui.hyperlink_to("Open Ko-fi webhook settings", "https://ko-fi.com/manage/webhooks");
                        ui.add(egui::TextEdit::singleline(&mut kofi.verification).password(true).char_limit(256).hint_text("Ko-fi verification token").background_color(egui::Color32::from_rgb(32,32,39)).desired_width(f32::INFINITY));
                        ui.checkbox(&mut kofi.remember,"Remember on this computer (desktop wallet)");
                        if kofi.service_url.trim().is_empty() && kofi.code.trim().is_empty() && kofi_status.owner.is_empty(){ui.label("The shared Ko-fi service has not been configured for this build yet. Your app distributor needs to deploy it once; streamers do not need their own hosting account.");}
                        if ui.add_enabled((!kofi.service_url.trim().is_empty() && !kofi.verification.trim().is_empty()) || !kofi.code.trim().is_empty(),egui::Button::new("Connect Ko-fi")).clicked(){kofi.connect();}
                        ui.small("Connects while Rosorah is open. Missed donations appear in Activity without replaying alerts. New missed donations increase matching-currency goals once.");
                        ui.collapsing("Advanced · service setup",|ui|{
                            ui.label("For the app distributor or a self-hosted receiver. Use only a server you trust: it receives donation data.");
                            ui.add(egui::TextEdit::singleline(&mut kofi.service_url).hint_text("https://your-service.workers.dev").desired_width(f32::INFINITY));
                            ui.label("Legacy 0.23 receiver connection code (optional)");
                            ui.add(egui::TextEdit::singleline(&mut kofi.code).password(true).char_limit(8192));
                        });
                        if !kofi_status.webhook.is_empty() {
                            ui.separator();
                            ui.label("2. Copy this webhook address into Ko-fi, save it there, then send a test notification.");
                            if ui.button("Copy webhook address").clicked(){ui.ctx().copy_text(kofi_status.webhook.clone());}
                            ui.small(&kofi_status.webhook);
                            ui.label("3. Once the test arrives, enable live donations below.");
                            if kofi_status.live {
                                if ui.button("Pause donations / enter test mode").clicked(){kofi.live(false);}
                                ui.small("Pause before sending more Ko-fi test notifications: Ko-fi does not always distinguish tests from real payments.");
                            } else if ui.add_enabled(kofi_status.verified && kofi_status.connected,egui::Button::new("Enable live donations")).clicked(){kofi.live(true);}
                            ui.small("Test mode verifies delivery without increasing goals. Payments received while paused are discarded. Live donations and membership payments feed donation alerts and matching-currency goals; shop orders and refunds are not imported.");
                        }
                    }
                    Provider::PayPal => {
                        ui.strong("Connect PayPal through your tipping service");
                        ui.label("Choose StreamElements or Streamlabs and follow its PayPal setup. Direct PayPal notifications are not configured in this build.");
                        ui.horizontal(|ui| {for p in [Provider::StreamElements,Provider::Streamlabs] {if ui.button(p.name()).clicked(){setup.provider=p;setup.step=Step::Prepare;}}});
                    }
                }
            }
            Step::Customize => {
                ui.strong("Choose what viewers will see");
                ui.checkbox(&mut setup.show_name,"Show the donor's display name");
                ui.checkbox(&mut setup.show_amount,"Show the amount and currency");
                ui.checkbox(&mut setup.show_message,"Show the donor's message");
                ui.label("Keep messages hidden if you want to review them before they appear on stream.");
                ui.horizontal(|ui| {ui.label("Preview currency"); egui::ComboBox::from_id_salt("donation_currency").selected_text(&setup.currency).show_ui(ui,|ui| {
                    for code in ["USD","CAD","EUR","GBP","AUD","JPY"] {ui.selectable_value(&mut setup.currency,code.to_string(),code);}
                });});
                ui.label("Currencies stay separate. This choice does not convert donations or change your PayPal account currency.");
                ui.label("Donation text");
                ui.add(egui::TextEdit::multiline(&mut setup.template).char_limit(1000).desired_rows(3));
                ui.small("Placeholders: {user}, {amount}, {currency}, {message}");
            }
            Step::Preview => {
                ui.strong("Try your sample alert");
                ui.label(setup.preview());
                if ui.button("Preview sample in Rosorah").clicked() { preview=Some(setup.preview()); }
                ui.label("SIMULATION — no money is sent, no goal is increased, and this does not verify payment delivery. This text preview is not included in recordings.");
                ui.separator();
                ui.strong(if (setup.provider == Provider::Kofi && kofi_status.connected && kofi_status.live) || (setup.provider != Provider::Kofi && status.connected) {"Live event connection active"} else {"Sample ready · live connection not active"});
                ui.label("For media alerts on stream, add a video/GIF source with the Donation trigger, or use a Donation rule with a Play media action. Text preview alerts appear only in the app.");
            }
        }
    });
    ui.add_space(12.);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(setup.step != Step::Choose, egui::Button::new("Back"))
            .clicked()
        {
            setup.step = setup.step.back();
        }
        let label = "Next";
        if ui
            .add_enabled(setup.step != Step::Preview, egui::Button::new(label))
            .clicked()
        {
            setup.step = setup.step.next();
            setup.started = true;
        }
        ui.label("Changes save automatically. You can leave and resume here.");
    });
    ui.collapsing("Need help?",|ui| {
        ui.label("Can't sign in? Use account recovery on the provider's official website. Rosorah cannot reset that password.");
        ui.label("No donation alert? First check that the event connection says Connected and that the donation used the linked integration. Opening a website is not enough.");
        ui.label("Duplicate alerts? Connect the service that owns your donation page, rather than also connecting PayPal for the same payment.");
        ui.label("Never share passwords, private API keys, or bank details with a viewer or in a support log.");
    });
    (
        before != serde_json::to_string(setup).unwrap_or_default(),
        preview,
    )
}
