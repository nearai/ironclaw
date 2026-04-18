# LunarWing

The LunarWing project is a hard fork of the Ironclaw project originally developed by NearAI.

The LunarWing project team maintains AGPLv3 license forever as well as AGPLv3 license on its extensions, tools, and channels.

The LunarWing project was created to encoruage freedom. The LunarWing project is dedicated free open source infrastructure.

The LunarWing project adds real privacy respecting tools and channels, right out of the box. These include, but are not limited to:
* Gotify (Tool, WASM)
* Weechat (Channel, utilize weechat as an IRC client for an agent to communicate, WASM)
* DarkIRC (Channel, Darkfi WASM)
* XMPP with OMEMO (wasm channel, bridge service, and core code changes had to be made to accomodate properly)
* Persistent Codex Worker Container with optional support for ACP, persistent mounted storage, and more! (Custom Woker Container)

LunarWing developers actually care about your freedom and DO NOT subject users to proprietary chinese-national developed software. We do not support proprietary channels such as Slack, Telegram, or Discord. 

LunarWing is not affiliated with NearAI.

## Partial List of Brand New Additional Features which LunarWing introduces

* Specialized secret management wrapper scripts for both Postgres and LibSQL. See Secrets_Manager for more details.
* Optional systemd and openrc services for channel bridges (Please note that utilizing some of the new channel bridges currently breaks multi-tenancy in certain ways. This is still being worked on)
* Improved Scheduling System
* Support for external agentic coding tools developed independently from upstream.
* Better support for logging common errors which still plague the upstream project.

### Project Scope

Our scope is large and is mainly concerned with adding many essential features from Upstream which are still missing, including advanced health checking and self-repair mechanisms. The LunarWing team is more interested in providing useful features instead of support for proprietary chinese document editing tools. Our vision for LunarWing is expressed in our MANIFESTO.
