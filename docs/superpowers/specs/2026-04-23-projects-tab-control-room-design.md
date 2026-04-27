# Projects tab control-room redesign

Date: 2026-04-23
Branch: `feat/missions-ui-redesign`
Status: Approved design, pending implementation

## Goal

Bring the Projects tab up to the same product quality as the redesigned Missions tab without duplicating the Missions detail experience.

The current Projects drill-in mixes two different roles:
- a useful control-room overview for project missions and recent activity
- a legacy detail renderer that expands raw markdown / large thread dumps at the bottom of the page

That second role is the main problem. It creates a jarring quality drop compared with the dossier-style Missions tab.

## Approved direction

Use **Option 2**:
- Clicking a **mission card** in Projects should **switch to the Missions tab** and open the matching mission in the canonical Missions detail view.
- Clicking a **Recent Activity** item should **stay in Projects** and open a **polished thread detail** surface within Projects.
- The old Projects-specific mission-detail renderer and raw bottom-of-page dump behavior should be removed.

## Product model

After this change, each surface has a clear responsibility:

### Projects tab
The Projects tab is the **control room**.
It should help the user:
- scan project health
- see what missions exist in a project
- inspect recent execution activity
- triage threads quickly without leaving project context unless they explicitly open a mission

### Missions tab
The Missions tab is the **canonical mission workspace**.
It owns:
- mission prompt / brief rendering
- mission metadata
- mission progress
- approach history
- mission thread drill-in
- mission actions

This avoids maintaining two competing mission-detail UIs.

## Problems in the current Projects tab

### 1. Mission clicks use the wrong detail model
Projects currently opens a custom mission detail panel in `cr-detail` using older rendering patterns (`renderMissionDetailInCr`).
This causes a lower-quality experience than the main Missions tab and creates duplicated maintenance.

### 2. Recent Activity opens oversized raw detail at the bottom
Recent activity rows currently expand a legacy thread detail renderer in the bottom detail panel.
The output is technically accurate but visually uncurated:
- full-width markdown walls
- very large raw prompt bodies
- low hierarchy between summary and detail
- message dumps without enough visual grouping

### 3. Drill-in layout loses focus when detail opens
The current drill-in becomes a stacked page where the detail appears below the missions and activity lists.
This makes detail inspection feel like an accidental page append instead of a deliberate inspection state.

## UX requirements

### Mission cards in Projects
When the user clicks a mission card in the project drill-in:
1. Switch to the `missions` tab
2. Load/open the exact mission by id
3. Show the mission in the canonical dossier-style mission detail view
4. Preserve the expected back behavior inside Missions

This interaction should feel like “open this mission in its real workspace”.

### Recent Activity in Projects
When the user clicks an activity row in the project drill-in:
1. Stay in the Projects tab
2. Open a polished thread detail view for that thread
3. Keep the project context visible and understandable
4. Make it easy to close the detail and return to the project drill-in

This interaction should feel like “inspect this execution without leaving the control room”.

## Intended interaction design

### Projects drill-in default state
The project drill-in should show:
- project header
- project widget area (existing)
- mission list column
- recent activity column
- no expanded raw detail dump by default

### Projects drill-in thread inspection state
When a recent activity item is opened:
- the thread detail should appear as a **designed inspection surface**, not as a raw appended dump
- the surrounding drill-in should still feel like the current context
- the detail must have strong hierarchy and constrained reading width

Implementation may use one of these layouts:
- replace the lower detail region with a polished inspector panel, or
- show the detail as a side/contained panel within the drill-in

For this implementation, the important requirement is not the exact geometry but the behavior:
- it must feel intentional
- it must not look like a markdown dump appended to the page
- it must not compete with the Missions tab’s role

## Thread detail design requirements

The Projects thread detail should be optimized for inspection, not archival completeness.

### Required sections

#### Header
- back action to return to the project drill-in
- thread goal/title
- state badge
- optional relationship hint if tied to a mission

#### Meta strip
Compact metadata cards or chips for:
- thread type
- steps
- tokens
- cost
- created time
- completed time

#### Summary block
A lightweight summary or lead section should appear before message content when possible.
If no derived summary exists, use the goal/title and metadata to establish orientation.

#### Message timeline
Messages should render as a readable timeline/log, not a wall:
- role labels remain visible
- assistant / user / system visually differentiated
- large prompt bodies constrained in width
- markdown typography improved
- code blocks and inline code styled clearly
- long content broken into digestible sections with spacing

### Content handling rules
- Keep full fidelity of message content, but improve presentation.
- Do not expose raw JSON-looking blobs unless they are genuinely the content.
- Long markdown bodies should use readable containers with max width and vertical rhythm.
- The visual emphasis should be on understanding the thread, not dumping every token equally.

## Architecture / implementation plan shape

### Behavior changes

### Remove Projects mission detail renderer as a destination
`renderMissionDetailInCr()` should no longer be the default path for mission-card clicks in Projects.
Mission-card clicks should route to the Missions tab and reuse the existing mission-detail behavior.

### Keep Projects-specific thread detail renderer
`crOpenEngineThread()` should remain a Projects-owned interaction, but its rendering should be redesigned.
It should become a control-room thread inspector rather than a raw bottom dump.

### Navigation changes
A new helper should exist for “open mission in Missions tab by id” from non-Missions surfaces.
That helper should:
- switch tabs
- ensure Missions state is loaded
- open the matching mission detail

The control-room should call that helper instead of using its own mission renderer.

### Visual/style changes
Projects-specific styles should be added or updated so the thread detail feels aligned with the new Missions quality bar:
- clearer container hierarchy
- stronger section titles
- better spacing
- narrower prose width for long content
- better timeline/message treatment
- improved empty/loading/back states

## Out of scope
- Reworking the top-level project overview cards
- Changing the project widget system
- Redesigning the Missions tab again
- Changing engine API payload shape unless implementation reveals a true blocker
- Moving Recent Activity out of Projects into another tab

## Acceptance criteria

### Mission navigation
- From Projects drill-in, clicking a mission opens the same mission in the Missions tab
- The Projects tab no longer renders a separate mission detail surface for that click path

### Thread inspection quality
- From Projects drill-in, clicking a recent activity row opens a polished thread detail in Projects
- The detail no longer reads like a raw bottom-of-page dump
- Long markdown / prompt content is readable and visually structured

### Role clarity
- Projects clearly feels like a control room
- Missions clearly feels like the canonical mission workspace
- There is no duplicated mission-detail experience with conflicting quality levels

## Testing / verification expectations

Implementation should verify at least:
- mission click from Projects routes to Missions and opens the intended mission
- recent activity click stays in Projects and opens thread detail
- back action from thread detail returns cleanly to the project drill-in
- no broken hash/navigation behavior
- JS syntax check passes
- diff is whitespace-clean

## Files likely involved

Primary:
- `crates/ironclaw_gateway/static/js/surfaces/projects.js`
- `crates/ironclaw_gateway/static/styles/surfaces/projects.css` (if present)
- `crates/ironclaw_gateway/static/styles/surfaces/missions.css` only if shared patterns are intentionally reused
- `crates/ironclaw_gateway/static/index.html` if layout hooks need adjustment

Possible supporting files:
- routing/navigation helpers under `crates/ironclaw_gateway/static/js/core/`
- shared markdown rendering styles if thread-inspector typography should be reused elsewhere

## Implementation recommendation

Implement this as a focused Projects-surface cleanup:
1. add mission deep-link helper to Missions
2. replace Projects mission click behavior
3. redesign Projects thread detail
4. verify navigation and readability

That keeps the diff scoped and aligned with the approved product direction.
