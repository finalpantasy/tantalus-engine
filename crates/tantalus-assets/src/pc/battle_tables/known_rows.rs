use super::*;

fn push(
    checks: &mut Vec<KnownRowCheck>,
    table: &'static str,
    row: usize,
    source: &'static str,
    expected: String,
    actual: String,
) {
    checks.push(KnownRowCheck {
        table,
        row,
        passed: expected == actual,
        expected,
        actual,
        source,
    });
}

/// Every expected value below was copied from Memoria's CSV (never from the
/// reader's own output), and the row indexes are the CSV `Id` column.
pub(super) fn run(t: &BattleTables, checks: &mut Vec<KnownRowCheck>) {
    push(
        checks,
        "actions",
        0,
        "Data/Battle/Actions.csv",
        format!(
            "{:?}",
            ((0, 0, 0, 0, 0, 0, 0, 10), (0, 0, 0, 0, 0, 0, 0, 0))
        ),
        format!("{:?}", {
            let a = &t.actions[0];
            (
                (
                    a.info.sub_win,
                    a.info.cursor,
                    a.info.def_cur,
                    a.info.dead,
                    a.info.def_dead,
                    a.info.def_cam,
                    a.info.vfx_no,
                    a.vfx2,
                ),
                (
                    a.btl_ref.prog_no,
                    a.btl_ref.power,
                    a.btl_ref.attr,
                    a.btl_ref.rate,
                    a.category,
                    a.add_no,
                    a.mp,
                    a.kind,
                ),
            )
        }),
    );
    push(
        checks,
        "actions",
        1,
        "Data/Battle/Actions.csv",
        format!(
            "{:?}",
            ((1, 3, 1, 1, 0, 0, 9, 8), (10, 16, 0, 0, 71, 0, 6, 1))
        ),
        format!("{:?}", {
            let a = &t.actions[1];
            (
                (
                    a.info.sub_win,
                    a.info.cursor,
                    a.info.def_cur,
                    a.info.dead,
                    a.info.def_dead,
                    a.info.def_cam,
                    a.info.vfx_no,
                    a.vfx2,
                ),
                (
                    a.btl_ref.prog_no,
                    a.btl_ref.power,
                    a.btl_ref.attr,
                    a.btl_ref.rate,
                    a.category,
                    a.add_no,
                    a.mp,
                    a.kind,
                ),
            )
        }),
    );
    push(
        checks,
        "actions",
        2,
        "Data/Battle/Actions.csv",
        format!(
            "{:?}",
            ((1, 3, 1, 1, 0, 0, 162, 141), (10, 38, 0, 0, 71, 0, 10, 1))
        ),
        format!("{:?}", {
            let a = &t.actions[2];
            (
                (
                    a.info.sub_win,
                    a.info.cursor,
                    a.info.def_cur,
                    a.info.dead,
                    a.info.def_dead,
                    a.info.def_cam,
                    a.info.vfx_no,
                    a.vfx2,
                ),
                (
                    a.btl_ref.prog_no,
                    a.btl_ref.power,
                    a.btl_ref.attr,
                    a.btl_ref.rate,
                    a.category,
                    a.add_no,
                    a.mp,
                    a.kind,
                ),
            )
        }),
    );
    push(
        checks,
        "actions",
        3,
        "Data/Battle/Actions.csv",
        format!(
            "{:?}",
            ((1, 3, 1, 1, 0, 0, 144, 142), (10, 107, 0, 0, 71, 0, 22, 1))
        ),
        format!("{:?}", {
            let a = &t.actions[3];
            (
                (
                    a.info.sub_win,
                    a.info.cursor,
                    a.info.def_cur,
                    a.info.dead,
                    a.info.def_dead,
                    a.info.def_cam,
                    a.info.vfx_no,
                    a.vfx2,
                ),
                (
                    a.btl_ref.prog_no,
                    a.btl_ref.power,
                    a.btl_ref.attr,
                    a.btl_ref.rate,
                    a.category,
                    a.add_no,
                    a.mp,
                    a.kind,
                ),
            )
        }),
    );
    push(
        checks,
        "actions",
        4,
        "Data/Battle/Actions.csv",
        format!(
            "{:?}",
            ((1, 0, 1, 0, 0, 0, 32, 10), (103, 0, 0, 255, 71, 1, 14, 0))
        ),
        format!("{:?}", {
            let a = &t.actions[4];
            (
                (
                    a.info.sub_win,
                    a.info.cursor,
                    a.info.def_cur,
                    a.info.dead,
                    a.info.def_dead,
                    a.info.def_cam,
                    a.info.vfx_no,
                    a.vfx2,
                ),
                (
                    a.btl_ref.prog_no,
                    a.btl_ref.power,
                    a.btl_ref.attr,
                    a.btl_ref.rate,
                    a.category,
                    a.add_no,
                    a.mp,
                    a.kind,
                ),
            )
        }),
    );
    push(
        checks,
        "actions",
        100,
        "Data/Battle/Actions.csv",
        format!(
            "{:?}",
            ((4, 1, 1, 0, 0, 0, 138, 10), (103, 0, 0, 255, 70, 30, 14, 2))
        ),
        format!("{:?}", {
            let a = &t.actions[100];
            (
                (
                    a.info.sub_win,
                    a.info.cursor,
                    a.info.def_cur,
                    a.info.dead,
                    a.info.def_dead,
                    a.info.def_cam,
                    a.info.vfx_no,
                    a.vfx2,
                ),
                (
                    a.btl_ref.prog_no,
                    a.btl_ref.power,
                    a.btl_ref.attr,
                    a.btl_ref.rate,
                    a.category,
                    a.add_no,
                    a.mp,
                    a.kind,
                ),
            )
        }),
    );
    push(
        checks,
        "actions",
        191,
        "Data/Battle/Actions.csv",
        format!(
            "{:?}",
            ((0, 8, 0, 0, 0, 0, 224, 10), (63, 40, 128, 0, 6, 0, 50, 0))
        ),
        format!("{:?}", {
            let a = &t.actions[191];
            (
                (
                    a.info.sub_win,
                    a.info.cursor,
                    a.info.def_cur,
                    a.info.dead,
                    a.info.def_dead,
                    a.info.def_cam,
                    a.info.vfx_no,
                    a.vfx2,
                ),
                (
                    a.btl_ref.prog_no,
                    a.btl_ref.power,
                    a.btl_ref.attr,
                    a.btl_ref.rate,
                    a.category,
                    a.add_no,
                    a.mp,
                    a.kind,
                ),
            )
        }),
    );
    push(
        checks,
        "status_data",
        0,
        "Data/Battle/StatusData.csv (ImmunityProvided omits the status's own bit)",
        format!("{:?}", (2, 0, 0, 134217728, 4294967294u64)),
        format!("{:?}", {
            let s = &t.status_data[0];
            (
                s.priority,
                s.opr_cnt,
                s.conti_cnt,
                s.clear,
                s.invalid & !(1u32 << 0),
            )
        }),
    );
    push(
        checks,
        "status_data",
        1,
        "Data/Battle/StatusData.csv (ImmunityProvided omits the status's own bit)",
        format!("{:?}", (7, 20, 0, 98304, 65536)),
        format!("{:?}", {
            let s = &t.status_data[1];
            (
                s.priority,
                s.opr_cnt,
                s.conti_cnt,
                s.clear,
                s.invalid & !(1u32 << 1),
            )
        }),
    );
    push(
        checks,
        "status_data",
        2,
        "Data/Battle/StatusData.csv (ImmunityProvided omits the status's own bit)",
        format!("{:?}", (0, 0, 0, 0, 0)),
        format!("{:?}", {
            let s = &t.status_data[2];
            (
                s.priority,
                s.opr_cnt,
                s.conti_cnt,
                s.clear,
                s.invalid & !(1u32 << 2),
            )
        }),
    );
    push(
        checks,
        "status_data",
        5,
        "Data/Battle/StatusData.csv (ImmunityProvided omits the status's own bit)",
        format!("{:?}", (10, 0, 0, 0, 0)),
        format!("{:?}", {
            let s = &t.status_data[5];
            (
                s.priority,
                s.opr_cnt,
                s.conti_cnt,
                s.clear,
                s.invalid & !(1u32 << 5),
            )
        }),
    );
    push(
        checks,
        "status_data",
        16,
        "Data/Battle/StatusData.csv (ImmunityProvided omits the status's own bit)",
        format!("{:?}", (8, 10, 30, 0, 0)),
        format!("{:?}", {
            let s = &t.status_data[16];
            (
                s.priority,
                s.opr_cnt,
                s.conti_cnt,
                s.clear,
                s.invalid & !(1u32 << 16),
            )
        }),
    );
    push(
        checks,
        "status_data",
        31,
        "Data/Battle/StatusData.csv (ImmunityProvided omits the status's own bit)",
        format!("{:?}", (0, 0, 25, 0, 134217792)),
        format!("{:?}", {
            let s = &t.status_data[31];
            (
                s.priority,
                s.opr_cnt,
                s.conti_cnt,
                s.clear,
                s.invalid & !(1u32 << 31),
            )
        }),
    );
    push(
        checks,
        "status_sets",
        0,
        "Data/Battle/StatusSets.csv",
        format!("{:?}", 0u32),
        format!("{:?}", t.status_sets[0]),
    );
    push(
        checks,
        "status_sets",
        1,
        "Data/Battle/StatusSets.csv",
        format!("{:?}", 262144u32),
        format!("{:?}", t.status_sets[1]),
    );
    push(
        checks,
        "status_sets",
        2,
        "Data/Battle/StatusSets.csv",
        format!("{:?}", 256u32),
        format!("{:?}", t.status_sets[2]),
    );
    push(
        checks,
        "status_sets",
        3,
        "Data/Battle/StatusSets.csv",
        format!("{:?}", 65538u32),
        format!("{:?}", t.status_sets[3]),
    );
    push(
        checks,
        "status_sets",
        4,
        "Data/Battle/StatusSets.csv",
        format!("{:?}", 2147483649u32),
        format!("{:?}", t.status_sets[4]),
    );
    push(
        checks,
        "status_sets",
        5,
        "Data/Battle/StatusSets.csv",
        format!("{:?}", 2466448408u32),
        format!("{:?}", t.status_sets[5]),
    );
    push(
        checks,
        "status_sets",
        38,
        "Data/Battle/StatusSets.csv",
        format!("{:?}", 269485072u32),
        format!("{:?}", t.status_sets[38]),
    );
    push(
        checks,
        "items",
        0,
        "Data/Items/Items.csv (SA ids are stored +192)",
        format!("{:?}", (250, 0, 0, 1, 0, [0, 0, 0])),
        format!("{:?}", {
            let r = &t.items[0];
            (r.price, r.shape, r.color, r.eq_lv, r.bonus, r.ability)
        }),
    );
    push(
        checks,
        "items",
        1,
        "Data/Items/Items.csv (SA ids are stored +192)",
        format!("{:?}", (320, 1, 0, 1, 0, [101, 0, 0])),
        format!("{:?}", {
            let r = &t.items[1];
            (r.price, r.shape, r.color, r.eq_lv, r.bonus, r.ability)
        }),
    );
    push(
        checks,
        "items",
        2,
        "Data/Items/Items.csv (SA ids are stored +192)",
        format!("{:?}", (500, 1, 12, 2, 0, [102, 101, 0])),
        format!("{:?}", {
            let r = &t.items[2];
            (r.price, r.shape, r.color, r.eq_lv, r.bonus, r.ability)
        }),
    );
    push(
        checks,
        "items",
        3,
        "Data/Items/Items.csv (SA ids are stored +192)",
        format!("{:?}", (950, 1, 0, 3, 0, [254, 0, 0])),
        format!("{:?}", {
            let r = &t.items[3];
            (r.price, r.shape, r.color, r.eq_lv, r.bonus, r.ability)
        }),
    );
    push(
        checks,
        "items",
        4,
        "Data/Items/Items.csv (SA ids are stored +192)",
        format!("{:?}", (2300, 1, 6, 6, 0, [105, 107, 0])),
        format!("{:?}", {
            let r = &t.items[4];
            (r.price, r.shape, r.color, r.eq_lv, r.bonus, r.ability)
        }),
    );
    push(
        checks,
        "items",
        5,
        "Data/Items/Items.csv (SA ids are stored +192)",
        format!("{:?}", (6000, 1, 0, 9, 0, [101, 0, 0])),
        format!("{:?}", {
            let r = &t.items[5];
            (r.price, r.shape, r.color, r.eq_lv, r.bonus, r.ability)
        }),
    );
    push(
        checks,
        "items",
        255,
        "Data/Items/Items.csv (SA ids are stored +192)",
        format!("{:?}", (0, 0, 0, 0, 0, [0, 0, 0])),
        format!("{:?}", {
            let r = &t.items[255];
            (r.price, r.shape, r.color, r.eq_lv, r.bonus, r.ability)
        }),
    );
    push(
        checks,
        "item_use",
        0,
        "Data/Items/ItemEffects.csv",
        format!("{:?}", (0, 1, 1, 292, 0, 0, 74, 12, 0, 0, 0)),
        format!("{:?}", {
            let u = &t.item_use[0];
            (
                u.info.cursor,
                u.info.def_cur,
                u.info.sub_win,
                u.info.vfx_no,
                u.info.dead,
                u.info.def_dead,
                u.btl_ref.prog_no,
                u.btl_ref.power,
                u.btl_ref.rate,
                u.btl_ref.attr,
                u.status,
            )
        }),
    );
    push(
        checks,
        "item_use",
        1,
        "Data/Items/ItemEffects.csv",
        format!("{:?}", (0, 1, 1, 292, 0, 0, 74, 20, 0, 0, 0)),
        format!("{:?}", {
            let u = &t.item_use[1];
            (
                u.info.cursor,
                u.info.def_cur,
                u.info.sub_win,
                u.info.vfx_no,
                u.info.dead,
                u.info.def_dead,
                u.btl_ref.prog_no,
                u.btl_ref.power,
                u.btl_ref.rate,
                u.btl_ref.attr,
                u.status,
            )
        }),
    );
    push(
        checks,
        "item_use",
        12,
        "Data/Items/ItemEffects.csv",
        format!("{:?}", (0, 1, 1, 8, 0, 0, 69, 10, 0, 0, 0)),
        format!("{:?}", {
            let u = &t.item_use[12];
            (
                u.info.cursor,
                u.info.def_cur,
                u.info.sub_win,
                u.info.vfx_no,
                u.info.dead,
                u.info.def_dead,
                u.btl_ref.prog_no,
                u.btl_ref.power,
                u.btl_ref.rate,
                u.btl_ref.attr,
                u.status,
            )
        }),
    );
    push(
        checks,
        "item_use",
        16,
        "Data/Items/ItemEffects.csv",
        format!("{:?}", (0, 1, 1, 17, 1, 1, 72, 0, 0, 0, 256)),
        format!("{:?}", {
            let u = &t.item_use[16];
            (
                u.info.cursor,
                u.info.def_cur,
                u.info.sub_win,
                u.info.vfx_no,
                u.info.dead,
                u.info.def_dead,
                u.btl_ref.prog_no,
                u.btl_ref.power,
                u.btl_ref.rate,
                u.btl_ref.attr,
                u.status,
            )
        }),
    );
    push(
        checks,
        "item_use",
        31,
        "Data/Items/ItemEffects.csv",
        format!("{:?}", (0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)),
        format!("{:?}", {
            let u = &t.item_use[31];
            (
                u.info.cursor,
                u.info.def_cur,
                u.info.sub_win,
                u.info.vfx_no,
                u.info.dead,
                u.info.def_dead,
                u.btl_ref.prog_no,
                u.btl_ref.power,
                u.btl_ref.rate,
                u.btl_ref.attr,
                u.status,
            )
        }),
    );
    push(
        checks,
        "weapons",
        0,
        "Data/Items/Weapons.csv",
        format!("{:?}", (1, 0, "GEO_WEP_B1_011", 5, 12, 0, 0, 0, 0)),
        format!("{:?}", {
            let w = &t.weapons[0];
            (
                w.category,
                w.add_no,
                w.model_name.as_deref().unwrap_or(""),
                w.btl_ref.prog_no,
                w.btl_ref.power,
                w.btl_ref.attr,
                w.btl_ref.rate,
                w.offset[0],
                w.offset[1],
            )
        }),
    );
    push(
        checks,
        "weapons",
        1,
        "Data/Items/Weapons.csv",
        format!("{:?}", (5, 0, "GEO_WEP_B1_012", 1, 12, 0, 0, 17, 200)),
        format!("{:?}", {
            let w = &t.weapons[1];
            (
                w.category,
                w.add_no,
                w.model_name.as_deref().unwrap_or(""),
                w.btl_ref.prog_no,
                w.btl_ref.power,
                w.btl_ref.attr,
                w.btl_ref.rate,
                w.offset[0],
                w.offset[1],
            )
        }),
    );
    push(
        checks,
        "weapons",
        2,
        "Data/Items/Weapons.csv",
        format!("{:?}", (5, 9, "GEO_WEP_B1_013", 1, 14, 0, 20, 81, 166)),
        format!("{:?}", {
            let w = &t.weapons[2];
            (
                w.category,
                w.add_no,
                w.model_name.as_deref().unwrap_or(""),
                w.btl_ref.prog_no,
                w.btl_ref.power,
                w.btl_ref.attr,
                w.btl_ref.rate,
                w.offset[0],
                w.offset[1],
            )
        }),
    );
    push(
        checks,
        "weapons",
        3,
        "Data/Items/Weapons.csv",
        format!("{:?}", (5, 0, "GEO_WEP_B1_014", 1, 18, 0, 0, 17, 200)),
        format!("{:?}", {
            let w = &t.weapons[3];
            (
                w.category,
                w.add_no,
                w.model_name.as_deref().unwrap_or(""),
                w.btl_ref.prog_no,
                w.btl_ref.power,
                w.btl_ref.attr,
                w.btl_ref.rate,
                w.offset[0],
                w.offset[1],
            )
        }),
    );
    push(
        checks,
        "weapons",
        4,
        "Data/Items/Weapons.csv",
        format!("{:?}", (5, 18, "GEO_WEP_B1_015", 1, 30, 0, 35, -32, 200)),
        format!("{:?}", {
            let w = &t.weapons[4];
            (
                w.category,
                w.add_no,
                w.model_name.as_deref().unwrap_or(""),
                w.btl_ref.prog_no,
                w.btl_ref.power,
                w.btl_ref.attr,
                w.btl_ref.rate,
                w.offset[0],
                w.offset[1],
            )
        }),
    );
    push(
        checks,
        "weapons",
        5,
        "Data/Items/Weapons.csv",
        format!("{:?}", (5, 0, "GEO_WEP_B1_016", 1, 42, 0, 0, 5, 201)),
        format!("{:?}", {
            let w = &t.weapons[5];
            (
                w.category,
                w.add_no,
                w.model_name.as_deref().unwrap_or(""),
                w.btl_ref.prog_no,
                w.btl_ref.power,
                w.btl_ref.attr,
                w.btl_ref.rate,
                w.offset[0],
                w.offset[1],
            )
        }),
    );
    push(
        checks,
        "armor",
        0,
        "Data/Items/Armors.csv",
        format!("{:?}", (0, 5, 0, 3)),
        format!("{:?}", {
            let a = &t.armor[0];
            (a.p_def, a.p_ev, a.m_def, a.m_ev)
        }),
    );
    push(
        checks,
        "armor",
        1,
        "Data/Items/Armors.csv",
        format!("{:?}", (0, 7, 0, 5)),
        format!("{:?}", {
            let a = &t.armor[1];
            (a.p_def, a.p_ev, a.m_def, a.m_ev)
        }),
    );
    push(
        checks,
        "armor",
        2,
        "Data/Items/Armors.csv",
        format!("{:?}", (0, 10, 0, 7)),
        format!("{:?}", {
            let a = &t.armor[2];
            (a.p_def, a.p_ev, a.m_def, a.m_ev)
        }),
    );
    push(
        checks,
        "armor",
        3,
        "Data/Items/Armors.csv",
        format!("{:?}", (0, 13, 0, 9)),
        format!("{:?}", {
            let a = &t.armor[3];
            (a.p_def, a.p_ev, a.m_def, a.m_ev)
        }),
    );
    push(
        checks,
        "armor",
        5,
        "Data/Items/Armors.csv",
        format!("{:?}", (0, 16, 1, 16)),
        format!("{:?}", {
            let a = &t.armor[5];
            (a.p_def, a.p_ev, a.m_def, a.m_ev)
        }),
    );
    push(
        checks,
        "armor",
        135,
        "Data/Items/Armors.csv",
        format!("{:?}", (0, 4, 0, 0)),
        format!("{:?}", {
            let a = &t.armor[135];
            (a.p_def, a.p_ev, a.m_def, a.m_ev)
        }),
    );
    push(
        checks,
        "equip_bonus",
        0,
        "Data/Items/Stats.csv",
        format!("{:?}", (0, 0, 0, 0, 0, 0, 0, 0, 0)),
        format!("{:?}", {
            let e = &t.equip_bonus[0];
            (
                e.dex,
                e.str_,
                e.mgc,
                e.wpr,
                e.p_up_attr,
                e.invalid,
                e.absorb,
                e.half,
                e.weak,
            )
        }),
    );
    push(
        checks,
        "equip_bonus",
        2,
        "Data/Items/Stats.csv",
        format!("{:?}", (0, 0, 0, 1, 0, 0, 0, 0, 0)),
        format!("{:?}", {
            let e = &t.equip_bonus[2];
            (
                e.dex,
                e.str_,
                e.mgc,
                e.wpr,
                e.p_up_attr,
                e.invalid,
                e.absorb,
                e.half,
                e.weak,
            )
        }),
    );
    push(
        checks,
        "equip_bonus",
        3,
        "Data/Items/Stats.csv",
        format!("{:?}", (0, 0, 0, 0, 0, 0, 0, 16, 0)),
        format!("{:?}", {
            let e = &t.equip_bonus[3];
            (
                e.dex,
                e.str_,
                e.mgc,
                e.wpr,
                e.p_up_attr,
                e.invalid,
                e.absorb,
                e.half,
                e.weak,
            )
        }),
    );
    push(
        checks,
        "equip_bonus",
        4,
        "Data/Items/Stats.csv",
        format!("{:?}", (0, 1, 0, 0, 8, 0, 0, 0, 0)),
        format!("{:?}", {
            let e = &t.equip_bonus[4];
            (
                e.dex,
                e.str_,
                e.mgc,
                e.wpr,
                e.p_up_attr,
                e.invalid,
                e.absorb,
                e.half,
                e.weak,
            )
        }),
    );
    push(
        checks,
        "equip_bonus",
        5,
        "Data/Items/Stats.csv",
        format!("{:?}", (0, 0, 0, 1, 0, 0, 0, 0, 0)),
        format!("{:?}", {
            let e = &t.equip_bonus[5];
            (
                e.dex,
                e.str_,
                e.mgc,
                e.wpr,
                e.p_up_attr,
                e.invalid,
                e.absorb,
                e.half,
                e.weak,
            )
        }),
    );
    push(
        checks,
        "equip_bonus",
        175,
        "Data/Items/Stats.csv",
        format!("{:?}", (0, 0, 0, 0, 0, 0, 0, 0, 0)),
        format!("{:?}", {
            let e = &t.equip_bonus[175];
            (
                e.dex,
                e.str_,
                e.mgc,
                e.wpr,
                e.p_up_attr,
                e.invalid,
                e.absorb,
                e.half,
                e.weak,
            )
        }),
    );
    push(
        checks,
        "support_abilities",
        0,
        "Data/Characters/Abilities/AbilityGems.csv (Gems = capa_val)",
        format!("{:?}", 15),
        format!("{:?}", t.support_abilities[0].capa_val),
    );
    push(
        checks,
        "support_abilities",
        1,
        "Data/Characters/Abilities/AbilityGems.csv (Gems = capa_val)",
        format!("{:?}", 6),
        format!("{:?}", t.support_abilities[1].capa_val),
    );
    push(
        checks,
        "support_abilities",
        2,
        "Data/Characters/Abilities/AbilityGems.csv (Gems = capa_val)",
        format!("{:?}", 9),
        format!("{:?}", t.support_abilities[2].capa_val),
    );
    push(
        checks,
        "support_abilities",
        3,
        "Data/Characters/Abilities/AbilityGems.csv (Gems = capa_val)",
        format!("{:?}", 10),
        format!("{:?}", t.support_abilities[3].capa_val),
    );
    push(
        checks,
        "support_abilities",
        4,
        "Data/Characters/Abilities/AbilityGems.csv (Gems = capa_val)",
        format!("{:?}", 12),
        format!("{:?}", t.support_abilities[4].capa_val),
    );
    push(
        checks,
        "support_abilities",
        5,
        "Data/Characters/Abilities/AbilityGems.csv (Gems = capa_val)",
        format!("{:?}", 4),
        format!("{:?}", t.support_abilities[5].capa_val),
    );
    push(
        checks,
        "shops",
        0,
        "Data/Items/ShopItems.csv (255 terminator omitted)",
        format!(
            "{:?}",
            vec![
                1, 2, 16, 17, 57, 70, 88, 89, 102, 112, 114, 136, 137, 149, 150, 177
            ]
        ),
        format!(
            "{:?}",
            t.shops[0]
                .iter()
                .copied()
                .filter(|v| *v != 255)
                .collect::<Vec<u8>>()
        ),
    );
    push(
        checks,
        "shops",
        1,
        "Data/Items/ShopItems.csv (255 terminator omitted)",
        format!(
            "{:?}",
            vec![
                1, 2, 3, 17, 31, 57, 79, 89, 90, 102, 103, 115, 116, 138, 151, 178
            ]
        ),
        format!(
            "{:?}",
            t.shops[1]
                .iter()
                .copied()
                .filter(|v| *v != 255)
                .collect::<Vec<u8>>()
        ),
    );
    push(
        checks,
        "shops",
        2,
        "Data/Items/ShopItems.csv (255 terminator omitted)",
        format!(
            "{:?}",
            vec![
                80, 90, 104, 115, 116, 117, 139, 152, 178, 236, 240, 241, 242, 243, 244, 253
            ]
        ),
        format!(
            "{:?}",
            t.shops[2]
                .iter()
                .copied()
                .filter(|v| *v != 255)
                .collect::<Vec<u8>>()
        ),
    );
    push(
        checks,
        "shops",
        3,
        "Data/Items/ShopItems.csv (255 terminator omitted)",
        format!(
            "{:?}",
            vec![
                51, 58, 90, 103, 104, 115, 116, 117, 136, 138, 139, 152, 178, 236, 240, 241, 242,
                243, 244, 253
            ]
        ),
        format!(
            "{:?}",
            t.shops[3]
                .iter()
                .copied()
                .filter(|v| *v != 255)
                .collect::<Vec<u8>>()
        ),
    );
    push(
        checks,
        "shops",
        4,
        "Data/Items/ShopItems.csv (255 terminator omitted)",
        format!(
            "{:?}",
            vec![
                1, 2, 3, 18, 32, 51, 58, 71, 72, 73, 79, 80, 89, 90, 91, 104, 117, 118, 139, 150,
                151, 152, 153, 178, 179
            ]
        ),
        format!(
            "{:?}",
            t.shops[4]
                .iter()
                .copied()
                .filter(|v| *v != 255)
                .collect::<Vec<u8>>()
        ),
    );
    push(
        checks,
        "shops",
        5,
        "Data/Items/ShopItems.csv (255 terminator omitted)",
        format!(
            "{:?}",
            vec![
                33, 52, 58, 71, 72, 80, 91, 92, 104, 105, 117, 118, 119, 140, 153, 154, 179, 180
            ]
        ),
        format!(
            "{:?}",
            t.shops[5]
                .iter()
                .copied()
                .filter(|v| *v != 255)
                .collect::<Vec<u8>>()
        ),
    );
    push(
        checks,
        "synthesis",
        0,
        "Data/Items/Synthesis.csv (Shops list = bits of the shop byte, shop id 32 is bit 0)",
        format!("{:?}", (300, 7, vec![1, 2], 79)),
        format!("{:?}", {
            let m = &t.synthesis[0];
            (m.price, m.dst, m.src.to_vec(), m.shop)
        }),
    );
    push(
        checks,
        "synthesis",
        1,
        "Data/Items/Synthesis.csv (Shops list = bits of the shop byte, shop id 32 is bit 0)",
        format!("{:?}", (700, 8, vec![2, 2], 95)),
        format!("{:?}", {
            let m = &t.synthesis[1];
            (m.price, m.dst, m.src.to_vec(), m.shop)
        }),
    );
    push(
        checks,
        "synthesis",
        2,
        "Data/Items/Synthesis.csv (Shops list = bits of the shop byte, shop id 32 is bit 0)",
        format!("{:?}", (1000, 9, vec![2, 3], 92)),
        format!("{:?}", {
            let m = &t.synthesis[2];
            (m.price, m.dst, m.src.to_vec(), m.shop)
        }),
    );
    push(
        checks,
        "synthesis",
        3,
        "Data/Items/Synthesis.csv (Shops list = bits of the shop byte, shop id 32 is bit 0)",
        format!("{:?}", (2000, 10, vec![3, 3], 88)),
        format!("{:?}", {
            let m = &t.synthesis[3];
            (m.price, m.dst, m.src.to_vec(), m.shop)
        }),
    );
    push(
        checks,
        "synthesis",
        4,
        "Data/Items/Synthesis.csv (Shops list = bits of the shop byte, shop id 32 is bit 0)",
        format!("{:?}", (9000, 11, vec![3, 4], 112)),
        format!("{:?}", {
            let m = &t.synthesis[4];
            (m.price, m.dst, m.src.to_vec(), m.shop)
        }),
    );
    push(
        checks,
        "synthesis",
        63,
        "Data/Items/Synthesis.csv (Shops list = bits of the shop byte, shop id 32 is bit 0)",
        format!("{:?}", (50000, 98, vec![92, 12], 32)),
        format!("{:?}", {
            let m = &t.synthesis[63];
            (m.price, m.dst, m.src.to_vec(), m.shop)
        }),
    );
    push(
        checks,
        "commands",
        0,
        "Data/Characters/Commands.csv (kind 1: ability indexes command_abilities, per BattleHUD)",
        format!("{:?}", (0, 0)),
        format!("{:?}", {
            let c = &t.commands[0];
            (
                c.kind,
                if c.kind == 1 {
                    t.command_abilities[c.ability as usize] as u64
                } else {
                    c.ability
                },
            )
        }),
    );
    push(
        checks,
        "commands",
        1,
        "Data/Characters/Commands.csv (kind 1: ability indexes command_abilities, per BattleHUD)",
        format!("{:?}", (0, 176)),
        format!("{:?}", {
            let c = &t.commands[1];
            (
                c.kind,
                if c.kind == 1 {
                    t.command_abilities[c.ability as usize] as u64
                } else {
                    c.ability
                },
            )
        }),
    );
    push(
        checks,
        "commands",
        2,
        "Data/Characters/Commands.csv (kind 1: ability indexes command_abilities, per BattleHUD)",
        format!("{:?}", (0, 177)),
        format!("{:?}", {
            let c = &t.commands[2];
            (
                c.kind,
                if c.kind == 1 {
                    t.command_abilities[c.ability as usize] as u64
                } else {
                    c.ability
                },
            )
        }),
    );
    push(
        checks,
        "commands",
        3,
        "Data/Characters/Commands.csv (kind 1: ability indexes command_abilities, per BattleHUD)",
        format!("{:?}", (0, 178)),
        format!("{:?}", {
            let c = &t.commands[3];
            (
                c.kind,
                if c.kind == 1 {
                    t.command_abilities[c.ability as usize] as u64
                } else {
                    c.ability
                },
            )
        }),
    );
    push(
        checks,
        "commands",
        14,
        "Data/Characters/Commands.csv (kind 1: ability indexes command_abilities, per BattleHUD)",
        format!("{:?}", (2, 0)),
        format!("{:?}", {
            let c = &t.commands[14];
            (
                c.kind,
                if c.kind == 1 {
                    t.command_abilities[c.ability as usize] as u64
                } else {
                    c.ability
                },
            )
        }),
    );
    push(
        checks,
        "commands",
        15,
        "Data/Characters/Commands.csv (kind 1: ability indexes command_abilities, per BattleHUD)",
        format!("{:?}", (3, 190)),
        format!("{:?}", {
            let c = &t.commands[15];
            (
                c.kind,
                if c.kind == 1 {
                    t.command_abilities[c.ability as usize] as u64
                } else {
                    c.ability
                },
            )
        }),
    );
    push(
        checks,
        "commands",
        16,
        "Data/Characters/Commands.csv (kind 1: ability indexes command_abilities, per BattleHUD)",
        format!("{:?}", (1, 49)),
        format!("{:?}", {
            let c = &t.commands[16];
            (
                c.kind,
                if c.kind == 1 {
                    t.command_abilities[c.ability as usize] as u64
                } else {
                    c.ability
                },
            )
        }),
    );
    push(
        checks,
        "commands",
        17,
        "Data/Characters/Commands.csv (kind 1: ability indexes command_abilities, per BattleHUD)",
        format!("{:?}", (1, 1)),
        format!("{:?}", {
            let c = &t.commands[17];
            (
                c.kind,
                if c.kind == 1 {
                    t.command_abilities[c.ability as usize] as u64
                } else {
                    c.ability
                },
            )
        }),
    );
    push(
        checks,
        "commands",
        18,
        "Data/Characters/Commands.csv (kind 1: ability indexes command_abilities, per BattleHUD)",
        format!("{:?}", (1, 49)),
        format!("{:?}", {
            let c = &t.commands[18];
            (
                c.kind,
                if c.kind == 1 {
                    t.command_abilities[c.ability as usize] as u64
                } else {
                    c.ability
                },
            )
        }),
    );
    push(
        checks,
        "level_base",
        0,
        "Data/Characters/BaseStats.csv (Speed, Strength, Magic, Spirit, Gems)",
        format!("{:?}", (23, 21, 18, 23, 18)),
        format!("{:?}", {
            let b = &t.level_base[0];
            (b.dex, b.str_, b.mgc, b.wpr, b.cap)
        }),
    );
    push(
        checks,
        "level_base",
        1,
        "Data/Characters/BaseStats.csv (Speed, Strength, Magic, Spirit, Gems)",
        format!("{:?}", (16, 12, 24, 19, 14)),
        format!("{:?}", {
            let b = &t.level_base[1];
            (b.dex, b.str_, b.mgc, b.wpr, b.cap)
        }),
    );
    push(
        checks,
        "level_base",
        2,
        "Data/Characters/BaseStats.csv (Speed, Strength, Magic, Spirit, Gems)",
        format!("{:?}", (21, 14, 23, 17, 14)),
        format!("{:?}", {
            let b = &t.level_base[2];
            (b.dex, b.str_, b.mgc, b.wpr, b.cap)
        }),
    );
    push(
        checks,
        "level_base",
        3,
        "Data/Characters/BaseStats.csv (Speed, Strength, Magic, Spirit, Gems)",
        format!("{:?}", (18, 24, 12, 21, 17)),
        format!("{:?}", {
            let b = &t.level_base[3];
            (b.dex, b.str_, b.mgc, b.wpr, b.cap)
        }),
    );
    push(
        checks,
        "level_base",
        4,
        "Data/Characters/BaseStats.csv (Speed, Strength, Magic, Spirit, Gems)",
        format!("{:?}", (20, 20, 16, 22, 18)),
        format!("{:?}", {
            let b = &t.level_base[4];
            (b.dex, b.str_, b.mgc, b.wpr, b.cap)
        }),
    );
    push(
        checks,
        "level_base",
        11,
        "Data/Characters/BaseStats.csv (Speed, Strength, Magic, Spirit, Gems)",
        format!("{:?}", (24, 21, 19, 23, 10)),
        format!("{:?}", {
            let b = &t.level_base[11];
            (b.dex, b.str_, b.mgc, b.wpr, b.cap)
        }),
    );
    push(
        checks,
        "level_hp_mp+level_exp",
        0,
        "Data/Characters/Leveling.csv",
        format!("{:?}", (0, 250, 200)),
        format!("{:?}", {
            let h = &t.level_hp_mp[0];
            (t.level_exp[0], h.hp, h.mp)
        }),
    );
    push(
        checks,
        "level_hp_mp+level_exp",
        1,
        "Data/Characters/Leveling.csv",
        format!("{:?}", (16, 314, 206)),
        format!("{:?}", {
            let h = &t.level_hp_mp[1];
            (t.level_exp[1], h.hp, h.mp)
        }),
    );
    push(
        checks,
        "level_hp_mp+level_exp",
        2,
        "Data/Characters/Leveling.csv",
        format!("{:?}", (47, 382, 212)),
        format!("{:?}", {
            let h = &t.level_hp_mp[2];
            (t.level_exp[2], h.hp, h.mp)
        }),
    );
    push(
        checks,
        "level_hp_mp+level_exp",
        3,
        "Data/Characters/Leveling.csv",
        format!("{:?}", (101, 454, 219)),
        format!("{:?}", {
            let h = &t.level_hp_mp[3];
            (t.level_exp[3], h.hp, h.mp)
        }),
    );
    push(
        checks,
        "level_hp_mp+level_exp",
        4,
        "Data/Characters/Leveling.csv",
        format!("{:?}", (186, 530, 226)),
        format!("{:?}", {
            let h = &t.level_hp_mp[4];
            (t.level_exp[4], h.hp, h.mp)
        }),
    );
    push(
        checks,
        "level_hp_mp+level_exp",
        98,
        "Data/Characters/Leveling.csv",
        format!("{:?}", (6633167, 4524, 642)),
        format!("{:?}", {
            let h = &t.level_hp_mp[98];
            (t.level_exp[98], h.hp, h.mp)
        }),
    );
    push(
        checks,
        "learnable_abilities",
        0,
        "Data/Characters/Abilities/Zidane.csv (first 5 rows; SA ids stored +192)",
        format!(
            "{:?}",
            vec![(101, 40), (102, 40), (103, 40), (104, 35), (105, 50)]
        ),
        format!(
            "{:?}",
            t.learnable_abilities[0]
                .iter()
                .take(5)
                .map(|p| (p.id, p.max_ap))
                .collect::<Vec<_>>()
        ),
    );
    push(
        checks,
        "learnable_abilities",
        1,
        "Data/Characters/Abilities/Vivi.csv (first 5 rows; SA ids stored +192)",
        format!(
            "{:?}",
            vec![(25, 25), (26, 50), (27, 75), (28, 20), (29, 25)]
        ),
        format!(
            "{:?}",
            t.learnable_abilities[1]
                .iter()
                .take(5)
                .map(|p| (p.id, p.max_ap))
                .collect::<Vec<_>>()
        ),
    );
    push(
        checks,
        "learnable_abilities",
        2,
        "Data/Characters/Abilities/Garnet.csv (first 5 rows; SA ids stored +192)",
        format!(
            "{:?}",
            vec![(49, 20), (51, 35), (53, 30), (55, 30), (58, 30)]
        ),
        format!(
            "{:?}",
            t.learnable_abilities[2]
                .iter()
                .take(5)
                .map(|p| (p.id, p.max_ap))
                .collect::<Vec<_>>()
        ),
    );
    push(
        checks,
        "command_abilities",
        16,
        "Data/Characters/Commands.csv Abilities column",
        format!("{:?}", vec![49, 51, 53, 55, 58, 60, 62, 64]),
        format!("{:?}", {
            let c = &t.commands[16];
            t.command_abilities[c.ability as usize..c.ability as usize + c.count as usize]
                .iter()
                .map(|v| i64::from(*v))
                .collect::<Vec<i64>>()
        }),
    );
    push(
        checks,
        "command_abilities",
        17,
        "Data/Characters/Commands.csv Abilities column",
        format!(
            "{:?}",
            vec![1, 2, 3, 5, 7, 8, 9, 11, 12, 14, 15, 16, 17, 18, 19, 20]
        ),
        format!("{:?}", {
            let c = &t.commands[17];
            t.command_abilities[c.ability as usize..c.ability as usize + c.count as usize]
                .iter()
                .map(|v| i64::from(*v))
                .collect::<Vec<i64>>()
        }),
    );
    push(
        checks,
        "command_abilities",
        18,
        "Data/Characters/Commands.csv Abilities column",
        format!("{:?}", vec![49, 51, 53, 55, 58, 60, 62, 64]),
        format!("{:?}", {
            let c = &t.commands[18];
            t.command_abilities[c.ability as usize..c.ability as usize + c.count as usize]
                .iter()
                .map(|v| i64::from(*v))
                .collect::<Vec<i64>>()
        }),
    );
    push(
        checks,
        "command_abilities",
        19,
        "Data/Characters/Commands.csv Abilities column",
        format!(
            "{:?}",
            vec![
                1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 20, 21, 22, 23, 24
            ]
        ),
        format!("{:?}", {
            let c = &t.commands[19];
            t.command_abilities[c.ability as usize..c.ability as usize + c.count as usize]
                .iter()
                .map(|v| i64::from(*v))
                .collect::<Vec<i64>>()
        }),
    );
    push(
        checks,
        "command_abilities",
        20,
        "Data/Characters/Commands.csv Abilities column",
        format!("{:?}", vec![68, 66, 72, 74]),
        format!("{:?}", {
            let c = &t.commands[20];
            t.command_abilities[c.ability as usize..c.ability as usize + c.count as usize]
                .iter()
                .map(|v| i64::from(*v))
                .collect::<Vec<i64>>()
        }),
    );
    push(
        checks,
        "command_abilities",
        21,
        "Data/Characters/Commands.csv Abilities column",
        format!(
            "{:?}",
            vec![
                1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15, 16, 20, 21, 22, 23, 24
            ]
        ),
        format!("{:?}", {
            let c = &t.commands[21];
            t.command_abilities[c.ability as usize..c.ability as usize + c.count as usize]
                .iter()
                .map(|v| i64::from(*v))
                .collect::<Vec<i64>>()
        }),
    );
    // G08: Esuna cures Blind (Memoria CL 2024.11.17 restored it): action 10
    // adds status set 5, and Set 5 holds Silence, Blind, Confuse, Poison,
    // Sleep, Heat, Freeze, Mini and GradualPetrify.
    push(
        checks,
        "actions",
        10,
        "Data/Battle/Actions.csv Esuna AddStatus",
        "5".to_owned(),
        t.actions[10].add_no.to_string(),
    );
    push(
        checks,
        "status_sets",
        5,
        "Data/Battle/StatusSets.csv Set 5 (Esuna)",
        format!(
            "{}",
            [3u32, 4, 10, 16, 17, 24, 25, 28, 31]
                .iter()
                .fold(0u32, |m, b| m | 1 << b)
        ),
        t.status_sets[5].to_string(),
    );
}
