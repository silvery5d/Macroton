//! Read-only eligibility checks for moving from per-AppID Steam prefixes to a
//! single resident client. A positive result is a migration prerequisite, not
//! permission to submit a game request through another prefix.

use crate::plan::LaunchPlan;
use crate::profile::{CompatibilityProfile, LaunchMode, SteamCloudPolicy, WindowsPayloadRoot};

pub fn profile_blockers(
    resident: &CompatibilityProfile,
    target: &CompatibilityProfile,
    resident_plan: &LaunchPlan,
    target_plan: &LaunchPlan,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if resident.app_id == target.app_id {
        blockers.push("the two AppIDs must be different".to_owned());
    }
    if resident.launch_mode != LaunchMode::Steam || target.launch_mode != LaunchMode::Steam {
        blockers.push("both games must use launch=steam".to_owned());
    }
    if resident_plan.runtime_id != target_plan.runtime_id
        || resident_plan.runtime_fingerprint_sha256 != target_plan.runtime_fingerprint_sha256
        || resident_plan.runtime_fingerprint_sha256.is_none()
    {
        blockers.push("pinned Wine runtime identity differs or is unresolved".to_owned());
    }
    if resident.steam_client_mode != target.steam_client_mode {
        blockers.push("Steam client modes differ".to_owned());
    }
    // These settings are applied to the resident client or its children at
    // process creation; a later -applaunch request cannot replace them.
    if resident.renderer != target.renderer || resident.graphics_api != target.graphics_api {
        blockers.push("renderer or graphics API differs".to_owned());
    }
    if resident.environment != target.environment {
        blockers.push("profile environment differs; resident Steam children inherit the first game's environment".to_owned());
    }
    if resident.windows_payload_root != WindowsPayloadRoot::SteamInstalledTree
        || target.windows_payload_root != WindowsPayloadRoot::SteamInstalledTree
    {
        blockers.push("both games need an authenticated Steam installed tree in the shared prefix".to_owned());
    }
    for (label, profile) in [("resident", resident), ("target", target)] {
        if profile.steam_cloud.effective(!profile.cloud_files.is_empty())
            == SteamCloudPolicy::Unspecified
        {
            blockers.push(format!("{label} game's Steam Cloud policy is unreviewed"));
        }
    }
    blockers.extend(resident_plan.blockers.iter().map(|item| format!("resident: {item}")));
    blockers.extend(target_plan.blockers.iter().map(|item| format!("target: {item}")));
    blockers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan;
    use std::path::Path;

    fn profile(app_id: u32) -> CompatibilityProfile {
        CompatibilityProfile::parse(&format!(
            "schema=1\nappid={app_id}\nruntime=wine\nruntime_version=1\nrenderer=wined3d-opengl\ngraphics_api=d3d9\nlaunch=steam\nwindows_payload_root=steam-installed-tree\nsteam_install_dir=Game{app_id}\nexecutable=game.exe\nsteam_cloud=unsupported\nsteam_cloud_source=https://store.steampowered.com/app/{app_id}/\nsteam_cloud_reviewed_date=2026-10-07\n"
        )).unwrap()
    }

    #[test]
    fn compatible_profiles_need_the_same_client_process_settings() {
        let a = profile(100);
        let b = profile(200);
        let mut a_plan = plan::build_steam_client(&a, Path::new("/missing"));
        let mut b_plan = plan::build_steam_client(&b, Path::new("/missing"));
        a_plan.blockers.clear();
        b_plan.blockers.clear();
        a_plan.runtime_fingerprint_sha256 = Some("a".repeat(64));
        b_plan.runtime_fingerprint_sha256 = Some("a".repeat(64));
        assert!(profile_blockers(&a, &b, &a_plan, &b_plan).is_empty());

        b_plan.runtime_fingerprint_sha256 = Some("b".repeat(64));
        assert!(profile_blockers(&a, &b, &a_plan, &b_plan).iter()
            .any(|item| item.contains("runtime identity")));

        b_plan.runtime_fingerprint_sha256 = a_plan.runtime_fingerprint_sha256.clone();
        let mut b_changed = b.clone();
        b_changed.environment.insert("WINEDEBUG".into(), "+cloud".into());
        assert!(profile_blockers(&a, &b_changed, &a_plan, &b_plan).iter()
            .any(|item| item.contains("environment differs")));
    }
}
