//! 자동 감지기 선택은 `polling`·`ddp_audit`·`fanotify_feed` 를 절대 고르지 않는다 (T-fdet-auto-01) — 등록 순서와 무관하게, 볼륨 종류 3가지 모두.
use std::path::PathBuf;
use std::sync::Arc;

use mam_core::VolumeKind;
use mam_core::watch::remote::RemoteHub;
use mam_core::watch::remote::fake::{FakeConnector, FakeDdp};
use mam_core::watch::{
    ChangeDetector, DdpAudit, DetectorRegistry, FanotifyFeed, OsEvents, Polling, ScanOnly,
    WatchTarget,
};

fn ids(r: &DetectorRegistry, kind: VolumeKind) -> Vec<&'static str> {
    let t = WatchTarget {
        folder_id: 1,
        root: PathBuf::from("."),
        volume_kind: kind,
    };
    r.plan(&t, None, None)
        .candidates
        .iter()
        .map(|c| c.detector.id())
        .collect()
}

#[test]
fn t_fdet_auto_01_auto_resolves_to_os_events_then_scan_only_on_every_volume_kind_whatever_the_registry_order()
 {
    let hub = RemoteHub::new(Arc::new(FakeConnector::new(FakeDdp::new("/a"), "SHA256:x")));
    let ddp = || -> Arc<dyn ChangeDetector> { Arc::new(DdpAudit::new(Arc::clone(&hub))) };
    let ses = || -> Arc<dyn ChangeDetector> { Arc::new(FanotifyFeed::new(Arc::clone(&hub))) };
    let registries = [
        // 위험한 등록 순서: 새 감지기가 앞에 있다
        DetectorRegistry::new(
            vec![
                ses(),
                ddp(),
                Arc::new(Polling::new()),
                Arc::new(OsEvents),
                Arc::new(ScanOnly),
            ],
            &["os_events", "scan_only"],
        ),
        DetectorRegistry::new(
            vec![
                Arc::new(Polling::new()),
                ddp(),
                ses(),
                Arc::new(ScanOnly),
                Arc::new(OsEvents),
            ],
            &["os_events", "scan_only"],
        ),
        DetectorRegistry::platform_default_with(vec![ddp(), ses()]),
        DetectorRegistry::platform_default_with(vec![ses()]),
        DetectorRegistry::platform_default(),
    ];
    for r in &registries {
        for kind in [
            VolumeKind::Fixed,
            VolumeKind::Removable,
            VolumeKind::Network,
        ] {
            assert_eq!(
                ids(r, kind),
                vec!["os_events", "scan_only"],
                "{kind:?} {r:?}"
            );
        }
    }
}
