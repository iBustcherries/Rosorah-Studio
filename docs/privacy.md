---
layout: default
title: Privacy Policy
---

[Rosorah Studio](./)

# Rosorah Studio Privacy Policy

Effective date: October 18, 2026.

## About this policy

Rosorah Studio is a free, open-source desktop application for composing, recording, and streaming video, connecting streaming accounts, displaying chat, and running alerts and automations. This policy describes the official application's handling of information. Independent modified versions and independently operated servers may have different practices.

The project is maintained by the owner of the [iBustcherries/Rosorah-Studio repository](https://github.com/iBustcherries/Rosorah-Studio).
Privacy contact: [Justmoeislive@gmail.com](mailto:Justmoeislive@gmail.com).

## Information used by the app

Depending on the features you enable, Rosorah processes:

- Connected account and channel identifiers, display names, avatars, authorization tokens, and stream connection information.
- Broadcast titles, descriptions, schedules, categories, visibility settings, thumbnails, and other settings you choose to manage.
- Chat messages, usernames, timestamps, badges, emotes, moderation actions, and stream activity such as follows, subscriptions, raids, gifts, and paid messages.
- Donation event identifiers, donor names, amounts, currencies, timestamps, and messages supplied by your connected donation provider.
- Video and audio from the screens, games, cameras, microphones, desktop audio, media files, and browser sources you choose.
- Scenes, sources, automation rules, widget settings, goals, local history, backups, and other application preferences.

These data enable the features you select: account connection, broadcast management, chat, moderation, alerts, goals, recording, streaming, and restoring your workspace.

## Google and YouTube

Rosorah Studio uses YouTube API Services. When you connect YouTube, Rosorah requests permission to access and manage YouTube information needed for its available features. The current implementation requests the `youtube.force-ssl` scope.

Rosorah uses authorized YouTube data to identify your channel, retrieve and manage broadcasts and stream connections, update broadcast information and thumbnails, and perform supported chat or moderation actions that you request. Browser-based chat also loads YouTube's own website and may require a separate website sign-in.

Rosorah Studio's use and transfer to any other app of information received from Google APIs will adhere to the [Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy), including its Limited Use requirements.

Google user data is not sold, used for personalized advertising, or used to train generalized artificial intelligence or machine-learning models. It is used to provide the user-facing features described here. It is not shared with unrelated parties for their own purposes.

See [Google's Privacy Policy](https://policies.google.com/privacy) and the [YouTube Terms of Service](https://www.youtube.com/t/terms).

## Local storage and browser sessions

Projects, settings, recordings, backups, and retained activity data are stored on your computer. Saved account tokens use Windows Credential Manager or a Linux Secret Service wallet. Stream keys use the system wallet when available; a private local-file fallback is supported for stream keys.

Browser chat uses a persistent browser profile with cookies, website storage, and a cache on your computer. Disconnecting an API account does not necessarily sign you out of the website or erase this browser profile.

Local recordings and exported backups remain until you delete them. Project files and backups may contain names, messages, media references, and goal information. Share them only with people you intend to receive those contents.

## Network services and sharing

Connecting a platform sends requests and relevant information to that platform. Starting a stream sends your selected video and audio to the destinations you configure. Chat messages and moderation actions are sent to the platform you select. Content placed on a canvas can become visible to your audience or be included in recordings.

Browser sources and chat pages contact their own providers, which may set cookies and receive connection information such as your IP address. Their privacy policies apply to their services.

The optional publisher-operated Kick sign-in service runs on Cloudflare. It processes authorization codes or refresh tokens to complete sign-in and renewal. Its application code does not persist those tokens. The Kick activity relay processes channel and event information and relays events to connected clients without retaining an event history.

The optional Ko-fi receiver also runs on Cloudflare. It processes connection credentials and donation notifications, and retains bounded history to deliver missed donations. StreamElements and Streamlabs connections receive events from those providers. PayPal payments are handled through the payment or donation provider, rather than Rosorah collecting payment-card details or your PayPal password.

Hosting providers, including Cloudflare and GitHub for this website, may process connection metadata and operational logs under their own policies. Information you voluntarily submit in a support request is available to its recipients; GitHub issues are public.

## Retention

Local activity history is bounded to 5,000 events and 30 days in the current implementation. Saved projects, goal totals, recordings, and backups have separate lifetimes and may retain information until removed.

The Ko-fi receiver makes up to 1,000 recent donations from the previous seven days available for delivery. Expired event payloads and duplicate identifiers are cleaned up when subsequent notifications arrive; idle accounts are not guaranteed automatic deletion after seven days. Explicit server-side account deletion is therefore distinct from disconnecting the desktop app.

Kick activity is relayed live without a persisted event history. Operational logs are separate from event history and are subject to the hosting provider's settings and retention.

## Your choices and deletion

You can choose which accounts, sources, services, and destinations to use, stop streaming or recording, and disconnect integrations.

To revoke Rosorah's Google access, visit [Google Account connections](https://myaccount.google.com/connections) and remove Rosorah Studio. Revoking access prevents future authorized access; it does not delete recordings, backups, previously posted content, or other existing copies.

To remove local information, disconnect accounts, close the app and its browser helper, and delete the relevant application data, browser profile, recordings, and backups. Stored credentials can also be removed through your operating system's credential manager. Contact us for help locating the correct files.

For deletion of publisher-operated relay data, contact the privacy address above. We may ask for information needed to verify control of the affected account. Removing a Ko-fi webhook prevents future deliveries; reconnecting can register the account again. Content already sent to streaming platforms must be managed through those platforms.

## Security

Rosorah uses HTTPS for supported account-service connections and operating-system credential storage where described above. Access to your computer, exported files, browser profiles, and hosting accounts affects the security of your information. No storage or transmission method is guaranteed secure.

## Policy changes and contact

Changes will be published on this page with an updated effective date. For questions about information access, correction, deletion, or this policy, contact [Justmoeislive@gmail.com](mailto:Justmoeislive@gmail.com). Do not post tokens, passwords, payment information, or private account details in public GitHub issues.
