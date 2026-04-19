# LunarWing

## Secure, Privacy focused AI Agent

<img width="2048" height="2048" alt="darklogo" src="https://github.com/user-attachments/assets/28e6abcb-16fe-43e5-8c44-6d2d734c64f3" />

The LunarWing project is a hard fork of the Ironclaw project originally developed by NearAI. The fork was initially started in Febuary 2026 and continues to expand far beyond what NearAI's project is currently capable of.

LunarWing adds much needed features to the project. There are far too many improvements to merge them all upstream.

Meaning: Lunar because we believe in Lunarpunk. Wing because a wing is an extension of the body which allows flight.

The LunarWing project team maintains AGPLv3 license forever as well as AGPLv3 license on its extensions, tools, and channels.

The LunarWing project was created with free open source software in mind. The LunarWing project is dedicated free open source infrastructure.

The LunarWing project adds real privacy respecting tools and channels, right out of the box. These include, but are not limited to:
* Gotify (Tool, WASM, agents can send notifications via gotify)
* Weechat (Channel, utilize weechat as an IRC client for an agent to communicate, WASM)
* DarkIRC (Channel, Darkfi WASM)
* XMPP with OMEMO (wasm channel, bridge service, and core code changes had to be made to accomodate properly)
* Persistent Codex Worker Container with optional support for ACP via a specialized bridge, optional persistent mounted storage, and much more! (Custom Woker Container)

LunarWing developers actually care about your freedom. We do not care about proprietary chinese-national developed software. We actually care about self hostable communciation layers. We will NOT continue to develop or support proprietary channels such as Slack, Telegram, or Discord due to ethical reasons but also because it is a huge waste of time.

The LunarWing development team is ACTUALLY serious about security and privacy. The LunarWing development team is not a democracy and the core development team has the FINAL say to all proposed changes, without exception.

LunarWing and its core contributers are not affiliated with NearAI.

## Partial List of Brand New Additional Features which LunarWing introduces which are not in the upstream repository:

* Specialized secret management wrapper scripts for both Postgres and LibSQL. See Secrets_Manager for more details.
* Optional systemd and openrc services for channel bridges (Please note that utilizing some of the new channel bridges currently breaks multi-tenancy in certain ways. This is still being worked on)
* Improved Scheduling System
* Support for external agentic coding tools developed independently from upstream.
* Better support for logging common errors which still plague the upstream project.
* Self healing, advanced healthchecks for channel bridge services, the running LunarWing binary/daemon/service itself, and even optional self healing solutions for routines in the case of routine failures. 

## Additionally, there is a custom HTTP proxy for TensorZero routing setups with optimized tool_choice routing for open source coding agent applications.
### Project Scope

Our scope is large and is mainly concerned with adding many essential features from Upstream which are still missing, including advanced health checking and self-repair mechanisms. The LunarWing team is more interested in providing useful features instead of support for proprietary chinese document editing tools or other crapware. Our vision for LunarWing is expressed in our MANIFESTO.

#### Things to do in the immediate/short term:
* add ALL the rest of the custom tools, channels, scripts, etc to the repo in proper form
* Move repository to Organization
* add proper task tracking
* Adminstrative Framework required to begin expanding operations and properly allocate funds for external contractors
* External Security Audits at some point
* update MANIFESTO
* introduce more stable, better, codex apc/acpx support. 
* continue to work on and improve self-healing solution
* Introduce GPLv3 into deny.toml for rust cargo crate licensing compatibility
* Further testing of new, enhanced multiservice multitenant features
