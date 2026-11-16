//! Known-row checks for the tables in `player.rs`. Every expected value was
//! copied from the pinned Memoria reference (commit c716e1fa), never from the
//! reader's own output:
//!
//! - `Data/Characters/CommandSets.csv` (command menus, columns First, Second,
//!   TranceFirst, TranceSecond),
//! - `Data/Characters/DefaultEquipment.csv` (`-1` is `255` in the installed
//!   byte table),
//! - `Data/Characters/CharacterParameters.csv` (row, win pose, category, command
//!   set, equipment set per character),
//! - `Data/Items/InitialItems.csv`,
//! - `Memoria/Data/Characters/CharacterSerialNumber.cs` (enum order),
//! - `Memoria/Assets/Text/CharacterNamesFormatter.cs` (default names),
//! - `Global/ff9play.cs` and `Memoria/Data/Characters/CharacterOldIndex.cs`
//!   constants.

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

pub(super) fn run(t: &BattleTables, checks: &mut Vec<KnownRowCheck>) {
    let p = &t.player;

    // CommandSets.csv: (id, normal first, second, trance first, second).
    const MENUS: [(usize, [u8; 2], [u8; 2]); 14] = [
        (0, [2, 25], [2, 26]),
        (1, [22, 13], [23, 13]),
        (2, [16, 17], [18, 17]),
        (3, [30, 31], [30, 31]),
        (4, [3, 27], [12, 27]),
        (5, [8, 24], [9, 24]),
        (6, [19, 20], [21, 20]),
        (7, [28, 15], [29, 15]),
        (8, [2, 0], [2, 0]),
        (14, [32, 34], [32, 34]),
        (15, [33, 34], [33, 34]),
        (16, [41, 0], [41, 0]),
        (17, [44, 0], [44, 0]),
        (19, [42, 0], [42, 0]),
    ];
    for (id, normal, trance) in MENUS {
        push(
            checks,
            "command_menus",
            id,
            "Data/Characters/CommandSets.csv",
            format!("{normal:?} {trance:?}"),
            p.command_menus
                .get(id)
                .map(|r| format!("{:?} {:?}", r.normal, r.trance))
                .unwrap_or_default(),
        );
    }

    // DefaultEquipment.csv: weapon, head, wrist, armor, accessory.
    const EQUIP: [[u8; 5]; 16] = [
        [1, 112, 88, 149, 255],
        [70, 112, 255, 149, 255],
        [57, 255, 255, 150, 255],
        [16, 137, 255, 177, 255],
        [31, 136, 102, 178, 255],
        [79, 255, 255, 255, 255],
        [64, 114, 90, 150, 232],
        [41, 255, 89, 155, 194],
        [0, 112, 255, 255, 255],
        [16, 112, 88, 149, 255],
        [16, 255, 255, 150, 255],
        [26, 138, 104, 179, 192],
        [17, 114, 88, 151, 255],
        [26, 142, 105, 181, 212],
        [17, 112, 255, 150, 255],
        [255, 255, 255, 255, 255],
    ];
    for (id, expected) in EQUIP.iter().enumerate() {
        push(
            checks,
            "default_equipment",
            id,
            "Data/Characters/DefaultEquipment.csv",
            format!("{expected:?}"),
            p.default_equipment
                .get(id)
                .map(|r| format!("{:?}", [r.weapon, r.head, r.wrist, r.armor, r.accessory]))
                .unwrap_or_default(),
        );
    }

    // CharacterParameters.csv: per character id (row, win pose, category,
    // command set, equipment set). Slots 0..4 are characters 0..4; slots 5..8
    // start as Cinna, Marcus, Blank and Beatrix (characters 8..11) because the
    // installed table shares slots (`GetCharID3`), which `PLAYER` also
    // declares (`KUINA = CINNA = 5`, `EIKO = MARCUS = 6`,
    // `SALAMANDER = BLANK = 7`, `BEATRIX = 8`).
    const SLOTS: [(usize, [u8; 5]); 9] = [
        (0, [1, 1, 9, 0, 0]),
        (1, [0, 1, 5, 1, 1]),
        (2, [0, 1, 6, 2, 2]),
        (3, [1, 1, 5, 3, 3]),
        (4, [0, 1, 6, 4, 4]),
        (5, [1, 1, 21, 8, 8]),
        (6, [1, 1, 21, 10, 9]),
        (7, [1, 1, 21, 12, 10]),
        (8, [1, 1, 22, 14, 11]),
    ];
    for (slot, expected) in SLOTS {
        push(
            checks,
            "player_slots",
            slot,
            "Data/Characters/CharacterParameters.csv",
            format!("{expected:?}"),
            p.player_slots
                .get(slot)
                .map(|r| {
                    format!(
                        "{:?}",
                        [r.row, r.win_pose, r.category, r.menu_type, r.equipment_set]
                    )
                })
                .unwrap_or_default(),
        );
    }

    // CharacterSerialNumber enum order (Zidane's installed value 1 is
    // ZIDANE_SWORD; Memoria's constructor passes ZIDANE_DAGGER and both are
    // replaced by `FF9Play_GetSerialID`, which this reader does not evaluate).
    for (slot, serial) in [
        (1usize, 2u8),
        (2, 3),
        (3, 7),
        (4, 12),
        (5, 14),
        (6, 15),
        (7, 16),
        (8, 18),
    ] {
        push(
            checks,
            "player_slots.serial_no",
            slot,
            "Memoria/Data/Characters/CharacterSerialNumber.cs",
            serial.to_string(),
            p.player_slots
                .get(slot)
                .map(|r| r.serial_no.to_string())
                .unwrap_or_default(),
        );
    }

    // InitialItems.csv (the installed literal also ends with a 255 x 0 entry).
    const ITEMS: [(u16, u8); 7] = [
        (236, 7),
        (237, 2),
        (238, 2),
        (240, 2),
        (247, 2),
        (249, 1),
        (253, 1),
    ];
    for (index, (id, count)) in ITEMS.iter().enumerate() {
        push(
            checks,
            "initial_items",
            index,
            "Data/Items/InitialItems.csv",
            format!("{id} x{count}"),
            p.initial_items
                .get(index)
                .map(|r| format!("{} x{}", r.id, r.count))
                .unwrap_or_default(),
        );
    }

    // ff9play.cs constants.
    for (name, value) in [
        ("FF9PLAY_DEF_LEVEL", 1),
        ("FF9PLAY_DEF_GIL", 500),
        ("FF9PLAY_DEF_AT", 10),
        ("FF9PLAY_NAME_MAX", 10),
        ("FF9PLAY_FLD_STATUS_MAX", 7),
        ("FF9PLAY_EQUIP_ID_NONE", 255),
        ("FF9PLAY_HP_MAX", 9999),
        ("FF9PLAY_MP_MAX", 999),
    ] {
        push(
            checks,
            "ff9play_constants",
            0,
            "Global/ff9play.cs",
            format!("{name}={value}"),
            format!(
                "{name}={}",
                p.ff9play_constants
                    .get(name)
                    .map_or("missing".to_owned(), |v| v.to_string())
            ),
        );
    }
    push(
        checks,
        "new_game.gil",
        0,
        "Global/ff9play.cs FF9PLAY_DEF_GIL",
        "Some(500)".to_owned(),
        format!("{:?}", p.new_game.gil),
    );

    // CharacterNamesFormatter.cs: US names by character id, read through the
    // menu_type -> character id function. Language keys US, GR, JP.
    const US: [&str; 12] = [
        "Zidane", "Vivi", "Dagger", "Steiner", "Freya", "Quina", "Eiko", "Amarant", "Cinna",
        "Marcus", "Blank", "Beatrix",
    ];
    let names_for = |language: &str| p.default_names.iter().find(|n| n.language == language);
    for (menu_type, char_id) in p
        .menu_type_char_ids
        .iter()
        .enumerate()
        .take(16)
        .map(|(m, c)| (m, *c))
    {
        let expected = usize::try_from(char_id).ok().and_then(|i| US.get(i));
        push(
            checks,
            "default_names.English(US)",
            menu_type,
            "Memoria/Assets/Text/CharacterNamesFormatter.cs",
            expected.map(|s| (*s).to_owned()).unwrap_or_default(),
            names_for("English(US)")
                .and_then(|n| n.names.get(menu_type))
                .cloned()
                .unwrap_or_default(),
        );
    }
    for (language, menu_type, expected) in [
        ("German", 2usize, "Lili"),
        ("German", 7, "Mahagon"),
        ("Japanese", 0, "ジタン"),
        ("Japanese", 7, "サラマンダー"),
    ] {
        push(
            checks,
            "default_names.other",
            menu_type,
            "Memoria/Assets/Text/CharacterNamesFormatter.cs",
            format!("{language} {expected}"),
            format!(
                "{language} {}",
                names_for(language)
                    .and_then(|n| n.names.get(menu_type))
                    .cloned()
                    .unwrap_or_default()
            ),
        );
    }

    // Text bindings against the file names Memoria's exporter writes.
    for (index, (name, mes)) in [
        ("ItemName", "/Item/itm_name.mes"),
        ("ItemHelpDescription", "/Item/itm_help.mes"),
        ("ItemBattleDescription", "/Item/itm_btl.mes"),
        ("ActionAbilityName", "/Ability/aa_name.mes"),
        ("SupportAbilityName", "/Ability/sa_name.mes"),
        ("CommandName", "/Command/com_name.mes"),
        ("CommandHelpDescription", "/Command/com_help.mes"),
        ("ImportantItemName", "/KeyItem/imp_name.mes"),
    ]
    .into_iter()
    .enumerate()
    {
        // Memoria's TranslationExporter writes `<lang>/Item/Itm_Name.mes`,
        // `Itm_Help.mes`, `Itm_Btl.mes`, `Ability/AA_Name.mes`,
        // `Ability/SA_Name.mes`, `Command/Com_Name.mes`, `Com_Help.mes` and
        // `KeyItem/Imp_Name.mes`; names compare without case.
        push(
            checks,
            "text_bindings",
            index,
            "Memoria/Assets/Export/TranslationExporter.cs",
            format!("{name} {}", mes.to_ascii_lowercase()),
            p.text_bindings
                .iter()
                .find(|b| b.accessor == name)
                .map(|b| format!("{name} {}", b.mes.to_ascii_lowercase()))
                .unwrap_or_default(),
        );
    }
}
