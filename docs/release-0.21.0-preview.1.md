# Rosorah Studio 0.21.0-preview.1

Close Rosorah, extract the whole folder, then launch run-studio.sh.

## Broadcast and services
- Streaming services are directly visible in Broadcast. Each connection has an enable toggle, Main/Vertical canvas selection and a Connection button. No disclosure/caret menu hides routing.
- Destination controls removed from the canvas previews.
- Add service supports existing Twitch/YouTube connections plus additional Kick, TikTok and Custom RTMP connections. Configure server and stream key, name the connection, assign its canvas, and enable it before Go live. Up to 16 additional connections.
- Additional connections appear in Streaming services settings as well. Existing account-managed Twitch/YouTube keys and sessions are retained.
- Kick/TikTok/custom connections in this preview use stream keys only. Their sign-in option is explicitly unavailable. Kick account sign-in still needs Rosorah developer-app registration and a suitable confidential token exchange. TikTok Login Kit alone does not establish LIVE ingest access; automatic LIVE linking is not implemented.
- TikTok requires an account with access to an encoder server/key. This build cannot grant that access. Get the exact server/key from the creator dashboard.
- Stream-key output does not connect Kick/TikTok chat or activity. Existing Twitch/YouTube chat is unchanged.
- Connections on the same canvas share an encoder; different canvases have separate encoders. Duplicate enabled endpoints are rejected. Connection changes are disabled during a live stream. No automatic go-live occurs.

## Requested spacing and interaction changes
- Chat history fills the space above the actual composer height. The editor still wraps up to five lines and then scrolls.
- Vertical mixer strips are fixed-width, adjacent, and wrap only when necessary.
- Drag a layer by its named row; click selects it. Visibility and context controls remain separate. Locked layers cannot be dragged.
- Material Studio styling and Roboto retained. Current panel arrangements remain intact.

See VALIDATION.md for verification and limits. Live platform authentication/streaming and Fedora performance require testing. Donation-provider support retains the limitations of prior previews.
