use super::*;

/// Redacted counts for monolithic installation snapshots imported from rc1.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Rc1SnapshotMigrationReport {
    pub sources_migrated: usize,
    pub sources_unchanged: usize,
    pub manifests_migrated: usize,
    pub manifests_unchanged: usize,
    pub installations_migrated: usize,
    pub installations_unchanged: usize,
}

impl Rc1SnapshotMigrationReport {
    fn merge(&mut self, other: Self) {
        self.sources_migrated = self.sources_migrated.saturating_add(other.sources_migrated);
        self.sources_unchanged = self
            .sources_unchanged
            .saturating_add(other.sources_unchanged);
        self.manifests_migrated = self
            .manifests_migrated
            .saturating_add(other.manifests_migrated);
        self.manifests_unchanged = self
            .manifests_unchanged
            .saturating_add(other.manifests_unchanged);
        self.installations_migrated = self
            .installations_migrated
            .saturating_add(other.installations_migrated);
        self.installations_unchanged = self
            .installations_unchanged
            .saturating_add(other.installations_unchanged);
    }
}

impl ExtensionInstallationStore {
    /// Compile the monolithic snapshot written by `ironclaw-v1.0.0-rc.1`
    /// into the compatibility rows consumed by the normalized v2 bootstrap.
    ///
    /// The source snapshot is deliberately retained for rollback. A restart
    /// may therefore see it after the normalized rows already exist; exact
    /// matches are no-ops, while a divergent compatibility or v2 target fails
    /// closed instead of silently choosing one history.
    pub(super) async fn bootstrap_from_rc1_snapshot(
        &self,
    ) -> Result<Rc1SnapshotMigrationReport, ExtensionInstallationError> {
        let snapshot_path = child_path(&self.root, "state.json")?;
        let marker_path = child_path(&self.root, ".migration/rc1-snapshot-v1.complete.json")?;
        self.bootstrap_from_rc1_snapshot_at(&snapshot_path, &marker_path)
            .await
    }

    /// Import an rc1 snapshot from a release-specific authority outside this
    /// store's normalized root. Hosted rc1 placed one snapshot under each
    /// tenant's `/system/extensions/.installations/state.json`; 1.1 owns one
    /// global normalized installation authority. Each source gets an adjacent
    /// hashed completion record under the target root, and the source is kept
    /// intact for rollback.
    pub async fn import_rc1_snapshot_at(
        &mut self,
        snapshot_path: &VirtualPath,
    ) -> Result<Rc1SnapshotMigrationReport, ExtensionInstallationError> {
        let source_key = sha256_digest_token(snapshot_path.as_str().as_bytes());
        let marker_path = child_path(
            &self.root,
            &format!(".migration/rc1-hosted-snapshot-{source_key}.complete.json"),
        )?;
        let report = self
            .bootstrap_from_rc1_snapshot_at(snapshot_path, &marker_path)
            .await?;
        // The core importer writes compatibility rows. Re-run the normalized
        // compiler/repair chain even when the source marker already exists so
        // a crash between those stages resumes safely.
        if report.sources_migrated > 0 || report.sources_unchanged > 0 {
            self.bootstrap_v2_from_compatibility_rows().await?;
            self.repair_interrupted_v2_leases().await?;
            self.repair_removed_v2_children().await?;
            self.repair_compatibility_views().await?;
        }
        self.rc1_snapshot_report.merge(report);
        Ok(report)
    }

    pub fn rc1_snapshot_migration_report(&self) -> Rc1SnapshotMigrationReport {
        self.rc1_snapshot_report
    }

    async fn bootstrap_from_rc1_snapshot_at(
        &self,
        snapshot_path: &VirtualPath,
        marker_path: &VirtualPath,
    ) -> Result<Rc1SnapshotMigrationReport, ExtensionInstallationError> {
        let Some(snapshot) =
            self.filesystem
                .get(snapshot_path)
                .await
                .map_err(store_unavailable(
                    "load rc1 extension installation snapshot",
                ))?
        else {
            return Ok(Rc1SnapshotMigrationReport::default());
        };
        let source_digest = sha256_digest_token(&snapshot.entry.body);
        let wire: Rc1WireState = snapshot.entry.parse_json().map_err(|error| {
            corrupt_row(
                "deserialize rc1 extension installation snapshot",
                snapshot_path,
                error,
            )
        })?;
        let manifest_count = wire.manifests.len();
        let installation_count = wire.installations.len();
        if let Some(marker) = self
            .filesystem
            .get(marker_path)
            .await
            .map_err(store_unavailable(
                "load rc1 extension snapshot migration marker",
            ))?
        {
            let marker: Rc1SnapshotMigrationMarker =
                marker.entry.parse_json().map_err(|error| {
                    corrupt_row(
                        "deserialize rc1 extension snapshot migration marker",
                        marker_path,
                        error,
                    )
                })?;
            if marker.source_digest == source_digest {
                return Ok(Rc1SnapshotMigrationReport {
                    sources_unchanged: 1,
                    manifests_unchanged: manifest_count,
                    installations_unchanged: installation_count,
                    ..Rc1SnapshotMigrationReport::default()
                });
            }
            return Err(invalid_installation_error(
                "rc1 extension snapshot changed after its migration completed",
            ));
        }
        let mut manifests = std::collections::BTreeMap::new();
        for wire_manifest in wire.manifests {
            let manifest = self.compile_rc1_manifest(wire_manifest)?;
            match manifests.get(manifest.extension_id()) {
                Some(existing) if existing == &manifest => {}
                Some(_) => {
                    return Err(invalid_installation_error(
                        "rc1 extension snapshot contains divergent duplicate manifests",
                    ));
                }
                None => {
                    manifests.insert(manifest.extension_id().clone(), manifest);
                }
            }
        }

        let mut installations = std::collections::BTreeMap::new();
        for installation in wire.installations {
            match installations.get(installation.installation_id()) {
                Some(existing) if existing == &installation => {}
                Some(_) => {
                    return Err(invalid_installation_error(
                        "rc1 extension snapshot contains divergent duplicate installations",
                    ));
                }
                None => {
                    installations.insert(installation.installation_id().clone(), installation);
                }
            }
        }

        for installation in installations.values() {
            let manifest = manifests.get(installation.extension_id()).ok_or_else(|| {
                invalid_installation_error(format!(
                    "rc1 installation {} has no manifest in its snapshot",
                    installation.installation_id()
                ))
            })?;
            validate_installation_against_one_manifest(manifest, installation)?;

            if let Some((core, _)) = self
                .load_v2_installation_record(installation.installation_id())
                .await?
                && (!core.is_visible()
                    || self.reconstruct_v2_installation(&core).await? != *installation)
            {
                return Err(invalid_installation_error(format!(
                    "rc1 installation {} conflicts with normalized v2 state",
                    installation.installation_id()
                )));
            }

            self.import_rc1_compatibility_row(
                &self.installation_path(installation.installation_id())?,
                entry_for_installation(installation)?,
                parse_installation_entry,
                installation,
            )
            .await?;
        }

        for manifest in manifests.values() {
            let v2_records = self
                .load_v2_records_by_extension(manifest.extension_id())
                .await?;
            for core in v2_records {
                if core.manifest.into_manifest_record()? != *manifest {
                    return Err(invalid_installation_error(format!(
                        "rc1 manifest {} conflicts with normalized v2 state",
                        manifest.extension_id()
                    )));
                }
            }
            self.import_rc1_compatibility_row(
                &self.manifest_path(manifest.extension_id())?,
                entry_for_manifest(manifest)?,
                |entry, path| self.parse_manifest_entry(entry, path),
                manifest,
            )
            .await?;
        }
        let marker = Rc1SnapshotMigrationMarker { source_digest };
        let marker_entry =
            Entry::bytes(serde_json::to_vec(&marker).map_err(invalid_installation_error)?);
        match self
            .filesystem
            .put(marker_path, marker_entry, CasExpectation::Absent)
            .await
        {
            Ok(_) => {}
            Err(FilesystemError::VersionMismatch { .. }) => {
                let current = self
                    .filesystem
                    .get(marker_path)
                    .await
                    .map_err(store_unavailable("verify rc1 snapshot migration marker"))?
                    .ok_or_else(|| {
                        store_unavailable_error(
                            "rc1 snapshot migration marker disappeared during verification",
                        )
                    })?;
                let current: Rc1SnapshotMigrationMarker =
                    current.entry.parse_json().map_err(|error| {
                        corrupt_row(
                            "deserialize rc1 extension snapshot migration marker",
                            marker_path,
                            error,
                        )
                    })?;
                if current != marker {
                    return Err(invalid_installation_error(
                        "rc1 extension snapshot migration marker conflicts",
                    ));
                }
            }
            Err(error) => {
                return Err(store_unavailable(
                    "write rc1 extension snapshot migration marker",
                )(error));
            }
        }
        Ok(Rc1SnapshotMigrationReport {
            sources_migrated: 1,
            manifests_migrated: manifest_count,
            installations_migrated: installation_count,
            ..Rc1SnapshotMigrationReport::default()
        })
    }

    fn compile_rc1_manifest(
        &self,
        wire: WireManifestRecord,
    ) -> Result<ExtensionManifestRecord, ExtensionInstallationError> {
        if wire.resolved.is_some() {
            return wire.into_manifest_record();
        }
        ExtensionManifestRecord::from_toml_with_root_binding(
            wire.raw_toml,
            wire.source.into_manifest_source(),
            &self.host_ports,
            wire.manifest_hash,
            &self.contracts,
            PackageRootBinding::FabricateOnLoad,
        )
        .map(|record| {
            record
                .with_removal_cleanup_requirements(wire.removal_cleanup_requirements)
                .with_definition_retention(wire.definition_retention)
        })
    }

    async fn import_rc1_compatibility_row<T, P>(
        &self,
        path: &VirtualPath,
        entry: Entry,
        parse: P,
        expected: &T,
    ) -> Result<(), ExtensionInstallationError>
    where
        T: PartialEq,
        P: FnOnce(Entry, &VirtualPath) -> Result<T, ExtensionInstallationError>,
    {
        match self
            .filesystem
            .put(path, entry, CasExpectation::Absent)
            .await
        {
            Ok(_) => Ok(()),
            Err(FilesystemError::VersionMismatch { .. }) => {
                let current = self
                    .filesystem
                    .get(path)
                    .await
                    .map_err(store_unavailable("verify rc1 compatibility import"))?
                    .ok_or_else(|| {
                        store_unavailable_error(
                            "rc1 compatibility row disappeared during import verification",
                        )
                    })?;
                if parse(current.entry, path)? == *expected {
                    Ok(())
                } else {
                    Err(invalid_installation_error(
                        "rc1 extension snapshot conflicts with compatibility state",
                    ))
                }
            }
            Err(error) => Err(store_unavailable("import rc1 compatibility row")(error)),
        }
    }
}

/// Exact top-level shape written by the rc1 composition-owned snapshot
/// store. Its child wires are intentionally the current compatibility readers
/// so every retired field is handled in one place.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1WireState {
    manifests: Vec<WireManifestRecord>,
    installations: Vec<ExtensionInstallation>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Rc1SnapshotMigrationMarker {
    source_digest: String,
}
