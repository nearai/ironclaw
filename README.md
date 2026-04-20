# LunarWing

## Secure, Privacy focused AI Agent

### Our website:
[LunarWing](https://lunarwing.org/)

<img width="512" height="512" alt="darklogo" src="https://github.com/user-attachments/assets/28e6abcb-16fe-43e5-8c44-6d2d734c64f3" />

The LunarWing project is a hard fork of the Ironclaw project originally developed by NearAI. The fork was initially started in Febuary 2026 and continues to expand far beyond what NearAI's project is currently capable of.

LunarWing adds much needed features to the project. There are far too many improvements to merge them all upstream.

## Why "LunarWing"?: 

### `Lunar` - We are firm believers in the Lunarpunk philosophy. See our MANIFESTO for more information.

### `Wing` - Wings are extensions of the body which allow flight. We can soar and we believe we will soar to even greater heights in time. (Wings can often serve other purposes as well.) We are not required to remain on the ground. Our imagination has enabled us to innovate in this space, where others have not been able to.

The LunarWing project team maintains AGPLv3 license forever as well as AGPLv3 license on its extensions, tools, and channels.

The LunarWing project was created with free open source software in mind. The LunarWing project is dedicated free open source infrastructure.

The LunarWing project adds real privacy respecting tools and channels, with full secret support, right out of the box. These include, but are not limited to:
* Gotify (Tool, WASM, agents can send notifications via gotify)
* Weechat (Channel, utilize weechat as an IRC client for an agent to communicate, WASM)
* DarkIRC (Channel, Darkfi WASM)
* XMPP with OMEMO (wasm channel, bridge service, and core code changes had to be made to accomodate properly)
* Persistent Codex Worker Container with optional support for ACP via a specialized bridge, optional persistent mounted storage, and much more! (Custom Woker Container)

LunarWing developers actually care about your freedom as a user. This means that we simply do not support adding tools and channels to our official repository which we do not think allign with our values (see MANIFESTO). We do not force users to shy away from said tools and channels, but we will not be supporting them in our main monorepo here. All wasm tools and channels that develepors wish to create and maintain for LunarWing can be done so elsewhere. We simply do not have the time or patience or willingness to develop and support certain proprietary platforms for LunarWing. Especially not when we feel there is so much more important work to accomplish for this project. What we DO care about is self-hostable communciation layers. We will NOT continue to develop or support proprietary channels such as Slack, Telegram, or Discord due to ethical reasons but also because we feel that it is not our place to do so.

The LunarWing core development team is ACTUALLY serious about security and privacy, unlike the vast majority of "Claw" software.

LunarWing and its core contributers are not affiliated with NearAI.

## A Partial List of Brand New Additional Features which LunarWing introduces which are not in the upstream repository:

* Specialized secret management wrapper scripts for both Postgres (we've enhanced postgres with finer tuned controls in our project) and LibSQL. See Secrets_Manager for more details.
* Optional systemd and openrc services for Lunarwing, channel bridges, and Healthcheck services (Please note that utilizing some of the new channel bridges currently breaks multi-tenancy in certain ways. This is still being worked on)
* Improved Scheduling System designed by Ruffles
* Support for external agentic coding tools developed independently from upstream.
* Better support for logging common errors which still plague the upstream project.
* Self healing, advanced healthchecks for channel bridge services, the running LunarWing binary/daemon/service itself, and even optional self healing solutions for routines in the case of routine failures.
* Automated Testing Suite for development work
* Function calls, Inference, and feedback for models (see Tensorzero for examples)

## Additionally, we support custom HTTP proxies for TensorZero routing setups with optimized tool_choice routing for open source coding agent applications as well as other various purposes.
### The Project Scope:
Our scope is large and is mainly concerned with adding many essential features from Upstream which are still missing, including more advanced health checking and self-repair mechanisms. The LunarWing team is more interested in providing useful features instead of support for proprietary chinese document editing tools or other unecessary crapware. Our vision for LunarWing is expressed in our MANIFESTO.

#### Our core team utilizes a self-hoste Vikunja kanban board to keep track of tasks.
