//! Admission-reservation bookkeeping on `Inner`.
use super::*;

impl Inner {
    pub(super) fn reserve_admission(
        &mut self,
        run_id: TurnRunId,
        admission_class: TurnAdmissionClass,
        scope: &TurnScope,
        actor: &TurnActor,
        limit_provider: &dyn TurnAdmissionLimitProvider,
    ) -> Result<(), AdmissionRejection> {
        let buckets = admission_buckets(scope, actor, &admission_class);
        for bucket in &buckets {
            let limit = limit_provider
                .limit_for(bucket)
                .map_err(|_| AdmissionRejection::new(AdmissionRejectionReason::Unavailable))?;
            if let Some(max_active) = limit.max_active {
                let active_count = self.active_admission_count(bucket);
                if active_count >= max_active {
                    return Err(
                        AdmissionRejection::new(AdmissionRejectionReason::TenantLimit)
                            .with_capacity_denial(crate::TurnAdmissionCapacityDenial {
                                axis_kind: bucket.axis_kind,
                                bucket_kind: bucket.bucket_kind,
                                admission_class: bucket.admission_class.clone(),
                                limit: max_active,
                                active_count,
                                retry_after_ms: limit.retry_after_ms,
                            }),
                    );
                }
            }
        }
        self.admission_reservations.insert(
            run_id,
            TurnAdmissionReservationRecord {
                run_id,
                admission_class,
                buckets,
                released: false,
            },
        );
        Ok(())
    }

    fn active_admission_count(&self, bucket: &TurnAdmissionBucket) -> u64 {
        self.admission_reservations
            .values()
            .filter(|reservation| {
                !reservation.released
                    && reservation
                        .buckets
                        .iter()
                        .any(|reserved| reserved == bucket)
            })
            .count() as u64
    }

    pub(super) fn active_admission_reservations(&self) -> Vec<TurnAdmissionReservationRecord> {
        self.admission_reservations
            .values()
            .filter(|reservation| !reservation.released)
            .cloned()
            .collect()
    }

    pub(super) fn release_admission(&mut self, run_id: TurnRunId) {
        if let Some(reservation) = self.admission_reservations.get_mut(&run_id) {
            reservation.released = true;
        }
    }
}
