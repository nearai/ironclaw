# this is a test file to test the capability of live ironclaw, org "xyzorg"
#
# (setup is done manually, out of band — boot serve, set the xyzorg tenant,
#  install the capabilities, bootstrap the operator/director. not part of this file.)
#
# ============================================================================
# SECTION 1 — org + accounts
# ============================================================================
# 1. the org "xyzorg" is the serve tenant.
# 2. create director@xyzorg.com as OWNER.
# 3. create officer@xyzorg.com as a member (user).
# 4. director promotes officer to ADMIN:
#      PUT /api/webchat/v2/admin/users/officer@xyzorg.com/role  {"role":"admin"}  (director's bearer)
# 5. create members alice, bob, carl (role = member).
#
# ============================================================================
# SECTION 2 — assign per-user capabilities (allow-list = "only X, deny the rest")
# ============================================================================
# mechanism (works today): enumerate the cap list, then hide each one NOT allowed:
#   GET  /api/webchat/v2/settings/tools                            -> the capability_id list
#   PUT  /api/webchat/v2/admin/users/{user}/capabilities/{cap_id}  {"availability":"hidden"}  -> per non-allowed cap
#   (use settings/tools for cap-ids; /admin/extensions returns PACKAGE ids.)
# 6. alice: allow builtin.shell + web_search; hide every other cap.
# 7. bob:   allow gdrive + github;    hide every other cap.
# 8. carl:  hide every cap (deny all).
# 9. bob's tools are USER-KEYED — set bob's own key per provider ("set a secret"):
#      POST /api/reborn/product-auth/manual-token/setup    (bob's bearer; get challenge)
#      POST /api/reborn/product-auth/manual-token/submit   (bob's bearer; provide bob's key/PAT)
#      (gdrive = Google is OAuth-keyed instead ->
#       POST /api/webchat/v2/extensions/google-drive/setup/oauth/start)
#
# ============================================================================
# SECTION 3 — assert role privileges (owner > admin > member)
# ============================================================================
# 10. admin can create members, and can PROMOTE a member to admin:
#       PUT /api/webchat/v2/admin/users/{user}/role  {"role":"admin"}   (officer's bearer)
#     -> once admin, that user gets the ADMIN defaults: their prior per-user capability
#        limitations no longer apply (admins are not capped to a grant set).
# 11. admins + owner can access all capabilities (the default set); per-user
#     hide-limitations apply to MEMBERS only.
# 12. deletion guards (run while all exist; after each 403 assert the target still exists):
#       officer(admin)  deletes director(owner)   -> 403  (admin may not delete an owner)
#       officer(admin)  deletes another admin     -> 403  (admin may not delete a peer admin)
#       officer(admin)  deletes himself           -> 403  (no self-delete)
#       director(owner) deletes himself           -> 403  (single owner is protected)
#       director(owner) deletes officer(admin)    -> 204  (owner outranks admin)
# 13. admin may NOT change the OWNER's capabilities:
#       PUT /api/webchat/v2/admin/users/director@xyzorg.com/capabilities/{cap}  (officer's bearer)
#       -> 403  (an admin may not modify an owner's grants)
#     (the org has exactly ONE owner for now — no second owner is created.)
#
# ============================================================================
# SECTION 4 — assert enforcement at dispatch (per-user tool surface)
# ============================================================================
# 14. run the SAME request as alice, bob, carl; assert each turn's trace / tool surface
#     contains ONLY that user's tools (alice: builtin.shell+web_search; bob: gdrive+github;
#     carl: none). assert at DISPATCH (the tools the model was offered), NOT from
#     settings/tools (the availability policy bites when the loop builds the surface).
# 15. bob's gdrive/github calls:
#      - key set       -> the tool turn completes.
#      - secret absent -> FINE: the user-keyed identity gate stops it GRACEFULLY
#                         (auth-required gate / unavailable), NOT a 500/crash.
#      - per-user isolation: bob's key is bob's alone; alice/carl never inherit it.
# 16. user-scoped approval prefs: any user may set "always approve" / "ask every time" /
#     "always deny" on a capability AVAILABLE to them
#       PUT /api/webchat/v2/settings/tools/{capability_id}   (the user's own bearer)
#     and CANNOT set it on an UNAVAILABLE capability -> rejected (403/404).
