//! Which Machine may order a certificate now.

use std::time::Duration;

use ployz_core::MachineId;

use crate::corrosion::CertificateRow;

/// Rank among Machine identifiers. Lowest id is 0 and may order immediately.
#[must_use]
pub(crate) fn machine_rank<'id>(
    this: &MachineId,
    machines: impl IntoIterator<Item = &'id MachineId>,
) -> usize {
    machines.into_iter().filter(|id| *id < this).count()
}

/// Whether this Machine should order now.
///
/// `step` must cover the HTTP-01 probe wait so a later rank cannot start
/// a competing order while an earlier rank is still presenting.
#[must_use]
pub(crate) fn rank_may_order(
    row: Option<&CertificateRow>,
    rank: usize,
    elapsed: Duration,
    step: Duration,
) -> bool {
    if row.and_then(CertificateRow::material).is_some() {
        return false;
    }
    let delay = step.saturating_mul(u32::try_from(rank).unwrap_or(u32::MAX));
    elapsed >= delay
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ployz_core::MachineId;

    use super::{machine_rank, rank_may_order};
    use crate::corrosion::{CertificateMaterial, CertificateRow};

    const STEP: Duration = Duration::from_secs(30);

    #[test]
    fn lowest_machine_id_is_rank_zero() {
        let low = MachineId::parse("a".repeat(32)).unwrap();
        let high = MachineId::parse("f".repeat(32)).unwrap();
        assert_eq!(machine_rank(&low, &[high, low]), 0);
        assert_eq!(machine_rank(&high, &[low]), 1);
        assert_eq!(machine_rank(&low, &[]), 0);
    }

    #[test]
    fn only_rank_zero_orders_immediately() {
        assert!(rank_may_order(None, 0, Duration::ZERO, STEP));
        assert!(!rank_may_order(
            None,
            1,
            Duration::from_secs(30) - Duration::from_millis(1),
            STEP
        ));
        assert!(rank_may_order(None, 1, Duration::from_secs(30), STEP));
        assert!(!rank_may_order(None, 2, Duration::from_secs(30), STEP));
        assert!(rank_may_order(None, 2, Duration::from_secs(60), STEP));
        assert!(!rank_may_order(
            None,
            1,
            Duration::from_secs(30),
            Duration::from_secs(120)
        ));
    }

    #[test]
    fn issued_material_means_nothing_to_do() {
        let material = CertificateMaterial::new("CERT", "KEY").unwrap();
        let row = CertificateRow::from_parts(Some(material), None);
        assert!(!rank_may_order(
            Some(&row),
            0,
            Duration::from_secs(60),
            STEP
        ));
    }
}
