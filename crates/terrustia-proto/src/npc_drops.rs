//! What NPCs drop when they die, transcribed from `ItemDropDatabase`.
//!
//! The game's loot system is a tree of rules. Most of it is one shape: roll a one-in-N chance, and
//! if it misses, roll the next thing instead. A skeleton's four possible weapons are one such
//! chain, and a demon eye's black lens sits behind its rarer drop the same way. That structure is
//! preserved here — a chain is tried in order and stops at the first success — because collapsing
//! it into independent rolls would make several drops far more common than they are.
//!
//! Every *unconditional* rule the game registers is here — all 248 of them, checked against
//! `ItemDropDatabase` with nothing disagreeing. Each is its own chain, because the game rolls them
//! independently of one another; only the rules the game itself chains are chained here.
//!
//! Deliberately absent, and worth knowing about: anything gated on a condition (a moon event, a
//! seed, a boss already beaten), the boss bags, and the "one from these options" rules. Those need
//! the condition system behind them, and guessing at them would be worse than leaving them out.

/// One thing an NPC might drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Drop {
    pub item: u16,
    /// A one-in-this chance. One means always.
    pub one_in: u32,
    pub min: i16,
    pub max: i16,
}

/// A run of alternatives, tried in order until one of them lands.
pub type DropChain = &'static [Drop];

/// What a type drops.
pub fn drops(npc_type: u16) -> &'static [DropChain] {
    match npc_type {
        1 => &[&[Drop {
            item: 1309,
            one_in: 10000,
            min: 1,
            max: 1,
        }]],
        2 => &[&[
            Drop {
                item: 236,
                one_in: 100,
                min: 1,
                max: 1,
            },
            Drop {
                item: 38,
                one_in: 3,
                min: 1,
                max: 1,
            },
        ]],
        3 => &[
            &[Drop {
                item: 216,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1304,
                one_in: 250,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5332,
                one_in: 1500,
                min: 1,
                max: 1,
            }],
        ],
        6 => &[
            &[Drop {
                item: 68,
                one_in: 3,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1309,
                one_in: 10000,
                min: 1,
                max: 1,
            }],
        ],
        7 => &[
            &[Drop {
                item: 68,
                one_in: 3,
                min: 1,
                max: 2,
            }],
            &[Drop {
                item: 69,
                one_in: 1,
                min: 3,
                max: 8,
            }],
            &[Drop {
                item: 1309,
                one_in: 10000,
                min: 1,
                max: 1,
            }],
        ],
        8 => &[
            &[Drop {
                item: 68,
                one_in: 3,
                min: 1,
                max: 2,
            }],
            &[Drop {
                item: 69,
                one_in: 1,
                min: 3,
                max: 8,
            }],
        ],
        9 => &[
            &[Drop {
                item: 68,
                one_in: 3,
                min: 1,
                max: 2,
            }],
            &[Drop {
                item: 69,
                one_in: 1,
                min: 3,
                max: 8,
            }],
        ],
        16 => &[
            &[Drop {
                item: 393,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1309,
                one_in: 10000,
                min: 1,
                max: 1,
            }],
        ],
        21 => &[
            &[
                Drop {
                    item: 954,
                    one_in: 100,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 955,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1166,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1274,
                    one_in: 500,
                    min: 1,
                    max: 1,
                },
            ],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        23 => &[
            &[Drop {
                item: 116,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5486,
                one_in: 100,
                min: 1,
                max: 1,
            }],
        ],
        24 => &[
            &[Drop {
                item: 1323,
                one_in: 20,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 244,
                one_in: 250,
                min: 1,
                max: 1,
            }],
        ],
        26 => &[&[
            Drop {
                item: 160,
                one_in: 200,
                min: 1,
                max: 1,
            },
            Drop {
                item: 161,
                one_in: 2,
                min: 1,
                max: 5,
            },
        ]],
        27 => &[&[
            Drop {
                item: 160,
                one_in: 200,
                min: 1,
                max: 1,
            },
            Drop {
                item: 161,
                one_in: 2,
                min: 1,
                max: 5,
            },
        ]],
        28 => &[&[
            Drop {
                item: 160,
                one_in: 200,
                min: 1,
                max: 1,
            },
            Drop {
                item: 161,
                one_in: 2,
                min: 1,
                max: 5,
            },
        ]],
        29 => &[&[
            Drop {
                item: 160,
                one_in: 200,
                min: 1,
                max: 1,
            },
            Drop {
                item: 161,
                one_in: 2,
                min: 1,
                max: 5,
            },
        ]],
        31 => &[
            &[Drop {
                item: 959,
                one_in: 450,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1307,
                one_in: 300,
                min: 1,
                max: 1,
            }],
        ],
        32 => &[
            &[Drop {
                item: 959,
                one_in: 450,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1307,
                one_in: 300,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5632,
                one_in: 150,
                min: 1,
                max: 1,
            }],
        ],
        43 => &[&[Drop {
            item: 210,
            one_in: 2,
            min: 1,
            max: 1,
        }]],
        44 => &[
            &[Drop {
                item: 1320,
                one_in: 20,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 88,
                one_in: 20,
                min: 1,
                max: 1,
            }],
        ],
        45 => &[&[Drop {
            item: 238,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        47 => &[&[Drop {
            item: 243,
            one_in: 75,
            min: 1,
            max: 1,
        }]],
        48 => &[&[Drop {
            item: 1516,
            one_in: 150,
            min: 1,
            max: 1,
        }]],
        49 => &[&[Drop {
            item: 18,
            one_in: 100,
            min: 1,
            max: 1,
        }]],
        51 => &[&[Drop {
            item: 18,
            one_in: 100,
            min: 1,
            max: 1,
        }]],
        52 => &[&[Drop {
            item: 251,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        53 => &[&[Drop {
            item: 239,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        54 => &[&[Drop {
            item: 260,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        58 => &[
            &[Drop {
                item: 393,
                one_in: 75,
                min: 1,
                max: 1,
            }],
            &[
                Drop {
                    item: 263,
                    one_in: 250,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 118,
                    one_in: 30,
                    min: 1,
                    max: 1,
                },
            ],
        ],
        60 => &[&[Drop {
            item: 1322,
            one_in: 150,
            min: 1,
            max: 1,
        }]],
        62 => &[&[Drop {
            item: 272,
            one_in: 35,
            min: 1,
            max: 1,
        }]],
        63 => &[
            &[Drop {
                item: 1303,
                one_in: 100,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 282,
                one_in: 1,
                min: 1,
                max: 4,
            }],
        ],
        64 => &[
            &[Drop {
                item: 1303,
                one_in: 100,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 282,
                one_in: 1,
                min: 1,
                max: 4,
            }],
        ],
        65 => &[&[
            Drop {
                item: 268,
                one_in: 20,
                min: 1,
                max: 1,
            },
            Drop {
                item: 319,
                one_in: 1,
                min: 1,
                max: 1,
            },
        ]],
        66 => &[
            &[Drop {
                item: 267,
                one_in: 1,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 272,
                one_in: 35,
                min: 1,
                max: 1,
            }],
        ],
        68 => &[&[Drop {
            item: 1169,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        69 => &[&[Drop {
            item: 323,
            one_in: 3,
            min: 1,
            max: 2,
        }]],
        71 => &[&[Drop {
            item: 327,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        73 => &[&[Drop {
            item: 362,
            one_in: 1,
            min: 1,
            max: 2,
        }]],
        75 => &[&[Drop {
            item: 501,
            one_in: 1,
            min: 1,
            max: 3,
        }]],
        77 => &[&[Drop {
            item: 723,
            one_in: 150,
            min: 1,
            max: 1,
        }]],
        79 => &[&[Drop {
            item: 527,
            one_in: 10,
            min: 1,
            max: 1,
        }]],
        80 => &[&[Drop {
            item: 528,
            one_in: 10,
            min: 1,
            max: 1,
        }]],
        81 => &[&[Drop {
            item: 996,
            one_in: 200,
            min: 1,
            max: 1,
        }]],
        83 => &[
            &[Drop {
                item: 996,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 6159,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        86 => &[
            &[Drop {
                item: 526,
                one_in: 1,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 856,
                one_in: 100,
                min: 1,
                max: 1,
            }],
        ],
        93 => &[&[Drop {
            item: 18,
            one_in: 100,
            min: 1,
            max: 1,
        }]],
        94 => &[
            &[Drop {
                item: 996,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 68,
                one_in: 3,
                min: 1,
                max: 1,
            }],
        ],
        98 => &[
            &[Drop {
                item: 996,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 522,
                one_in: 1,
                min: 2,
                max: 5,
            }],
        ],
        101 => &[
            &[Drop {
                item: 996,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 522,
                one_in: 1,
                min: 2,
                max: 5,
            }],
        ],
        102 => &[&[Drop {
            item: 263,
            one_in: 250,
            min: 1,
            max: 1,
        }]],
        104 => &[&[Drop {
            item: 485,
            one_in: 60,
            min: 1,
            max: 1,
        }]],
        109 => &[
            &[Drop {
                item: 1324,
                one_in: 10,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 4271,
                one_in: 10,
                min: 1,
                max: 1,
            }],
        ],
        110 => &[
            &[Drop {
                item: 682,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1321,
                one_in: 40,
                min: 1,
                max: 1,
            }],
        ],
        111 => &[&[
            Drop {
                item: 160,
                one_in: 200,
                min: 1,
                max: 1,
            },
            Drop {
                item: 161,
                one_in: 2,
                min: 1,
                max: 5,
            },
        ]],
        124 => &[&[Drop {
            item: 4818,
            one_in: 8,
            min: 1,
            max: 1,
        }]],
        125 => &[&[Drop {
            item: 1368,
            one_in: 10,
            min: 1,
            max: 1,
        }]],
        126 => &[&[Drop {
            item: 1369,
            one_in: 10,
            min: 1,
            max: 1,
        }]],
        143 => &[&[Drop {
            item: 593,
            one_in: 1,
            min: 5,
            max: 10,
        }]],
        144 => &[&[Drop {
            item: 593,
            one_in: 1,
            min: 5,
            max: 10,
        }]],
        145 => &[&[Drop {
            item: 593,
            one_in: 1,
            min: 5,
            max: 10,
        }]],
        147 => &[&[Drop {
            item: 1309,
            one_in: 10000,
            min: 1,
            max: 1,
        }]],
        150 => &[&[Drop {
            item: 18,
            one_in: 100,
            min: 1,
            max: 1,
        }]],
        151 => &[&[Drop {
            item: 1322,
            one_in: 50,
            min: 1,
            max: 1,
        }]],
        153 => &[&[Drop {
            item: 1328,
            one_in: 12,
            min: 1,
            max: 1,
        }]],
        154 => &[
            &[Drop {
                item: 1306,
                one_in: 100,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1253,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        156 => &[&[Drop {
            item: 1518,
            one_in: 50,
            min: 1,
            max: 1,
        }]],
        158 => &[
            &[Drop {
                item: 5597,
                one_in: 40,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1520,
                one_in: 40,
                min: 1,
                max: 1,
            }],
        ],
        159 => &[
            &[Drop {
                item: 5597,
                one_in: 40,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1520,
                one_in: 40,
                min: 1,
                max: 1,
            }],
        ],
        161 => &[
            &[Drop {
                item: 216,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1304,
                one_in: 250,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5332,
                one_in: 1500,
                min: 1,
                max: 1,
            }],
        ],
        162 => &[&[Drop {
            item: 5261,
            one_in: 450,
            min: 1,
            max: 1,
        }]],
        166 => &[&[Drop {
            item: 5261,
            one_in: 450,
            min: 1,
            max: 1,
        }]],
        167 => &[
            &[Drop {
                item: 393,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 879,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        169 => &[
            &[Drop {
                item: 1306,
                one_in: 100,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 726,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        170 => &[&[Drop {
            item: 4428,
            one_in: 100,
            min: 1,
            max: 1,
        }]],
        171 => &[&[Drop {
            item: 4428,
            one_in: 100,
            min: 1,
            max: 1,
        }]],
        172 => &[
            &[Drop {
                item: 754,
                one_in: 1,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 755,
                one_in: 1,
                min: 1,
                max: 1,
            }],
        ],
        173 => &[&[Drop {
            item: 1330,
            one_in: 3,
            min: 1,
            max: 1,
        }]],
        174 => &[&[Drop {
            item: 996,
            one_in: 200,
            min: 1,
            max: 1,
        }]],
        175 => &[&[Drop {
            item: 1265,
            one_in: 100,
            min: 1,
            max: 1,
        }]],
        176 => &[&[Drop {
            item: 209,
            one_in: 6,
            min: 1,
            max: 1,
        }]],
        179 => &[
            &[Drop {
                item: 996,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 6159,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        180 => &[&[Drop {
            item: 4428,
            one_in: 100,
            min: 1,
            max: 1,
        }]],
        181 => &[&[Drop {
            item: 1330,
            one_in: 3,
            min: 1,
            max: 1,
        }]],
        182 => &[
            &[Drop {
                item: 996,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1330,
                one_in: 3,
                min: 1,
                max: 1,
            }],
        ],
        183 => &[&[Drop {
            item: 996,
            one_in: 200,
            min: 1,
            max: 1,
        }]],
        185 => &[
            &[Drop {
                item: 393,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 951,
                one_in: 25,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5070,
                one_in: 1,
                min: 1,
                max: 2,
            }],
        ],
        186 => &[&[Drop {
            item: 40,
            one_in: 1,
            min: 1,
            max: 9,
        }]],
        190 => &[&[
            Drop {
                item: 236,
                one_in: 100,
                min: 1,
                max: 1,
            },
            Drop {
                item: 38,
                one_in: 3,
                min: 1,
                max: 1,
            },
        ]],
        191 => &[&[
            Drop {
                item: 236,
                one_in: 100,
                min: 1,
                max: 1,
            },
            Drop {
                item: 38,
                one_in: 3,
                min: 1,
                max: 1,
            },
        ]],
        192 => &[&[
            Drop {
                item: 236,
                one_in: 100,
                min: 1,
                max: 1,
            },
            Drop {
                item: 38,
                one_in: 3,
                min: 1,
                max: 1,
            },
        ]],
        193 => &[&[
            Drop {
                item: 236,
                one_in: 100,
                min: 1,
                max: 1,
            },
            Drop {
                item: 38,
                one_in: 3,
                min: 1,
                max: 1,
            },
        ]],
        194 => &[&[
            Drop {
                item: 236,
                one_in: 100,
                min: 1,
                max: 1,
            },
            Drop {
                item: 38,
                one_in: 3,
                min: 1,
                max: 1,
            },
        ]],
        197 => &[
            &[Drop {
                item: 1306,
                one_in: 100,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 393,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        198 => &[
            &[Drop {
                item: 2806,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2807,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2808,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1172,
                one_in: 1000,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1293,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2766,
                one_in: 7,
                min: 1,
                max: 2,
            }],
        ],
        201 => &[
            &[
                Drop {
                    item: 954,
                    one_in: 100,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 955,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1166,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1274,
                    one_in: 500,
                    min: 1,
                    max: 1,
                },
            ],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        202 => &[
            &[
                Drop {
                    item: 954,
                    one_in: 100,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 955,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1166,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1274,
                    one_in: 500,
                    min: 1,
                    max: 1,
                },
            ],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        203 => &[
            &[
                Drop {
                    item: 954,
                    one_in: 100,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 955,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1166,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1274,
                    one_in: 500,
                    min: 1,
                    max: 1,
                },
            ],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        204 => &[
            &[Drop {
                item: 1309,
                one_in: 10000,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 209,
                one_in: 2,
                min: 1,
                max: 1,
            }],
        ],
        206 => &[
            &[Drop {
                item: 1306,
                one_in: 100,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 726,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        207 => &[&[Drop {
            item: 3349,
            one_in: 8,
            min: 1,
            max: 1,
        }]],
        208 => &[&[Drop {
            item: 3548,
            one_in: 4,
            min: 30,
            max: 60,
        }]],
        216 => &[
            &[Drop {
                item: 905,
                one_in: 1000,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 855,
                one_in: 500,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 854,
                one_in: 250,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2584,
                one_in: 250,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3033,
                one_in: 125,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 672,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5460,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        217 => &[&[Drop {
            item: 1115,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        218 => &[&[Drop {
            item: 1116,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        219 => &[&[Drop {
            item: 1117,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        220 => &[&[Drop {
            item: 1118,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        221 => &[&[Drop {
            item: 1119,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        223 => &[
            &[Drop {
                item: 216,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1304,
                one_in: 250,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5332,
                one_in: 1500,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 282,
                one_in: 1,
                min: 1,
                max: 4,
            }],
        ],
        224 => &[
            &[Drop {
                item: 4057,
                one_in: 100,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 282,
                one_in: 1,
                min: 1,
                max: 4,
            }],
        ],
        225 => &[&[Drop {
            item: 1243,
            one_in: 45,
            min: 1,
            max: 1,
        }]],
        226 => &[
            &[Drop {
                item: 2806,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2807,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2808,
                one_in: 200,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1172,
                one_in: 1000,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1293,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2766,
                one_in: 7,
                min: 1,
                max: 2,
            }],
        ],
        227 => &[&[Drop {
            item: 3350,
            one_in: 8,
            min: 1,
            max: 1,
        }]],
        239 => &[&[Drop {
            item: 1330,
            one_in: 3,
            min: 1,
            max: 1,
        }]],
        240 => &[&[Drop {
            item: 1330,
            one_in: 3,
            min: 1,
            max: 1,
        }]],
        243 => &[
            &[Drop {
                item: 1519,
                one_in: 3,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2161,
                one_in: 1,
                min: 1,
                max: 1,
            }],
        ],
        244 => &[&[Drop {
            item: 662,
            one_in: 1,
            min: 30,
            max: 60,
        }]],
        250 => &[&[Drop {
            item: 1244,
            one_in: 15,
            min: 1,
            max: 1,
        }]],
        251 => &[
            &[Drop {
                item: 5239,
                one_in: 15,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5236,
                one_in: 15,
                min: 1,
                max: 1,
            }],
        ],
        253 => &[&[Drop {
            item: 5223,
            one_in: 60,
            min: 1,
            max: 1,
        }]],
        268 => &[&[Drop {
            item: 1332,
            one_in: 1,
            min: 2,
            max: 5,
        }]],
        269 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        270 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        271 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        272 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        273 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        274 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        275 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        276 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        277 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        278 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        279 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        280 => &[&[Drop {
            item: 1517,
            one_in: 300,
            min: 1,
            max: 1,
        }]],
        288 => &[&[Drop {
            item: 1508,
            one_in: 1,
            min: 1,
            max: 2,
        }]],
        294 => &[
            &[Drop {
                item: 959,
                one_in: 450,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1307,
                one_in: 300,
                min: 1,
                max: 1,
            }],
        ],
        295 => &[
            &[Drop {
                item: 959,
                one_in: 450,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1307,
                one_in: 300,
                min: 1,
                max: 1,
            }],
        ],
        296 => &[
            &[Drop {
                item: 959,
                one_in: 450,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1307,
                one_in: 300,
                min: 1,
                max: 1,
            }],
        ],
        301 => &[&[Drop {
            item: 5576,
            one_in: 10,
            min: 1,
            max: 1,
        }]],
        302 => &[&[Drop {
            item: 1309,
            one_in: 10000,
            min: 1,
            max: 1,
        }]],
        353 => &[&[Drop {
            item: 3352,
            one_in: 8,
            min: 1,
            max: 1,
        }]],
        368 => &[&[Drop {
            item: 2222,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        381 => &[&[Drop {
            item: 2860,
            one_in: 8,
            min: 8,
            max: 20,
        }]],
        382 => &[&[Drop {
            item: 2860,
            one_in: 8,
            min: 8,
            max: 20,
        }]],
        383 => &[&[Drop {
            item: 2860,
            one_in: 8,
            min: 8,
            max: 20,
        }]],
        385 => &[&[Drop {
            item: 2860,
            one_in: 8,
            min: 8,
            max: 20,
        }]],
        386 => &[&[Drop {
            item: 2860,
            one_in: 8,
            min: 8,
            max: 20,
        }]],
        389 => &[&[Drop {
            item: 2860,
            one_in: 8,
            min: 8,
            max: 20,
        }]],
        390 => &[
            &[Drop {
                item: 2860,
                one_in: 8,
                min: 8,
                max: 20,
            }],
            &[Drop {
                item: 2771,
                one_in: 30,
                min: 1,
                max: 1,
            }],
        ],
        395 => &[&[Drop {
            item: 6173,
            one_in: 50,
            min: 1,
            max: 1,
        }]],
        431 => &[
            &[Drop {
                item: 216,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1304,
                one_in: 250,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5332,
                one_in: 1500,
                min: 1,
                max: 1,
            }],
        ],
        432 => &[&[Drop {
            item: 40,
            one_in: 1,
            min: 1,
            max: 9,
        }]],
        441 => &[&[Drop {
            item: 3351,
            one_in: 8,
            min: 1,
            max: 1,
        }]],
        449 => &[
            &[
                Drop {
                    item: 954,
                    one_in: 100,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 955,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1166,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1274,
                    one_in: 500,
                    min: 1,
                    max: 1,
                },
            ],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        450 => &[
            &[
                Drop {
                    item: 954,
                    one_in: 100,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 955,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1166,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1274,
                    one_in: 500,
                    min: 1,
                    max: 1,
                },
            ],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        451 => &[
            &[
                Drop {
                    item: 954,
                    one_in: 100,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 955,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1166,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1274,
                    one_in: 500,
                    min: 1,
                    max: 1,
                },
            ],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        452 => &[
            &[
                Drop {
                    item: 954,
                    one_in: 100,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 955,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1166,
                    one_in: 200,
                    min: 1,
                    max: 1,
                },
                Drop {
                    item: 1274,
                    one_in: 500,
                    min: 1,
                    max: 1,
                },
            ],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        460 => &[&[Drop {
            item: 5227,
            one_in: 60,
            min: 1,
            max: 1,
        }]],
        462 => &[&[Drop {
            item: 5262,
            one_in: 60,
            min: 1,
            max: 1,
        }]],
        464 => &[&[Drop {
            item: 243,
            one_in: 75,
            min: 1,
            max: 1,
        }]],
        469 => &[&[Drop {
            item: 5260,
            one_in: 60,
            min: 1,
            max: 1,
        }]],
        476 => &[
            &[Drop {
                item: 52,
                one_in: 3,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1724,
                one_in: 3,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2353,
                one_in: 3,
                min: 5,
                max: 10,
            }],
            &[Drop {
                item: 1922,
                one_in: 3,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 678,
                one_in: 3,
                min: 3,
                max: 5,
            }],
            &[Drop {
                item: 1336,
                one_in: 3,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2676,
                one_in: 3,
                min: 2,
                max: 4,
            }],
            &[Drop {
                item: 2272,
                one_in: 3,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5395,
                one_in: 3,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 4986,
                one_in: 3,
                min: 69,
                max: 69,
            }],
        ],
        477 => &[&[Drop {
            item: 5237,
            one_in: 15,
            min: 1,
            max: 1,
        }]],
        480 => &[&[Drop {
            item: 3269,
            one_in: 25,
            min: 1,
            max: 1,
        }]],
        481 => &[
            &[Drop {
                item: 3094,
                one_in: 2,
                min: 40,
                max: 80,
            }],
            &[Drop {
                item: 4463,
                one_in: 20,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5543,
                one_in: 100,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 118,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        482 => &[
            &[Drop {
                item: 3086,
                one_in: 1,
                min: 5,
                max: 10,
            }],
            &[Drop {
                item: 6167,
                one_in: 80,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3109,
                one_in: 30,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 4400,
                one_in: 20,
                min: 1,
                max: 1,
            }],
        ],
        483 => &[
            &[Drop {
                item: 3086,
                one_in: 1,
                min: 5,
                max: 10,
            }],
            &[Drop {
                item: 6167,
                one_in: 80,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3109,
                one_in: 30,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 4400,
                one_in: 20,
                min: 1,
                max: 1,
            }],
        ],
        490 => &[
            &[Drop {
                item: 3212,
                one_in: 150,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3213,
                one_in: 200,
                min: 1,
                max: 1,
            }],
        ],
        491 => &[
            &[Drop {
                item: 905,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 855,
                one_in: 15,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 854,
                one_in: 15,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 2584,
                one_in: 15,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3033,
                one_in: 15,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 4471,
                one_in: 20,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 672,
                one_in: 10,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5460,
                one_in: 10,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3359,
                one_in: 10,
                min: 1,
                max: 1,
            }],
        ],
        498 => &[&[
            Drop {
                item: 18,
                one_in: 80,
                min: 1,
                max: 1,
            },
            Drop {
                item: 393,
                one_in: 80,
                min: 1,
                max: 1,
            },
            Drop {
                item: 3285,
                one_in: 15,
                min: 1,
                max: 1,
            },
        ]],
        508 => &[
            &[Drop {
                item: 323,
                one_in: 3,
                min: 1,
                max: 2,
            }],
            &[Drop {
                item: 3772,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        509 => &[
            &[Drop {
                item: 323,
                one_in: 3,
                min: 1,
                max: 2,
            }],
            &[Drop {
                item: 3772,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        513 => &[&[Drop {
            item: 3380,
            one_in: 2,
            min: 1,
            max: 2,
        }]],
        520 => &[&[Drop {
            item: 2860,
            one_in: 8,
            min: 8,
            max: 20,
        }]],
        524 => &[&[Drop {
            item: 3794,
            one_in: 10,
            min: 1,
            max: 3,
        }]],
        525 => &[
            &[Drop {
                item: 3794,
                one_in: 10,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 522,
                one_in: 3,
                min: 1,
                max: 3,
            }],
            &[Drop {
                item: 527,
                one_in: 15,
                min: 1,
                max: 1,
            }],
        ],
        526 => &[
            &[Drop {
                item: 3794,
                one_in: 10,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1332,
                one_in: 3,
                min: 1,
                max: 3,
            }],
            &[Drop {
                item: 527,
                one_in: 15,
                min: 1,
                max: 1,
            }],
        ],
        527 => &[
            &[Drop {
                item: 3794,
                one_in: 10,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 528,
                one_in: 15,
                min: 1,
                max: 1,
            }],
        ],
        528 => &[
            &[Drop {
                item: 2802,
                one_in: 25,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 6157,
                one_in: 40,
                min: 1,
                max: 1,
            }],
        ],
        529 => &[
            &[Drop {
                item: 2801,
                one_in: 25,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 6157,
                one_in: 40,
                min: 1,
                max: 1,
            }],
        ],
        532 => &[
            &[Drop {
                item: 3380,
                one_in: 1,
                min: 1,
                max: 3,
            }],
            &[Drop {
                item: 3771,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        536 => &[
            &[Drop {
                item: 3478,
                one_in: 1,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3479,
                one_in: 1,
                min: 1,
                max: 1,
            }],
        ],
        537 => &[&[Drop {
            item: 1309,
            one_in: 8000,
            min: 1,
            max: 1,
        }]],
        541 => &[&[Drop {
            item: 3783,
            one_in: 1,
            min: 1,
            max: 1,
        }]],
        542 => &[&[Drop {
            item: 319,
            one_in: 8,
            min: 1,
            max: 1,
        }]],
        543 => &[
            &[Drop {
                item: 319,
                one_in: 8,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 527,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        544 => &[
            &[Drop {
                item: 319,
                one_in: 8,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 527,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        545 => &[
            &[Drop {
                item: 319,
                one_in: 8,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 528,
                one_in: 25,
                min: 1,
                max: 1,
            }],
        ],
        550 => &[&[Drop {
            item: 3821,
            one_in: 8,
            min: 1,
            max: 1,
        }]],
        551 => &[&[Drop {
            item: 3866,
            one_in: 10,
            min: 1,
            max: 1,
        }]],
        564 => &[
            &[Drop {
                item: 3864,
                one_in: 7,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3867,
                one_in: 10,
                min: 1,
                max: 1,
            }],
        ],
        565 => &[
            &[Drop {
                item: 3864,
                one_in: 14,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3867,
                one_in: 10,
                min: 1,
                max: 1,
            }],
        ],
        576 => &[&[Drop {
            item: 3868,
            one_in: 10,
            min: 1,
            max: 1,
        }]],
        577 => &[
            &[Drop {
                item: 3856,
                one_in: 10,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 3868,
                one_in: 10,
                min: 1,
                max: 1,
            }],
        ],
        580 => &[
            &[Drop {
                item: 323,
                one_in: 3,
                min: 1,
                max: 2,
            }],
            &[Drop {
                item: 3772,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        581 => &[
            &[Drop {
                item: 323,
                one_in: 3,
                min: 1,
                max: 2,
            }],
            &[Drop {
                item: 3772,
                one_in: 50,
                min: 1,
                max: 1,
            }],
        ],
        582 => &[&[Drop {
            item: 323,
            one_in: 6,
            min: 1,
            max: 1,
        }]],
        586 => &[
            &[Drop {
                item: 4608,
                one_in: 2,
                min: 4,
                max: 6,
            }],
            &[Drop {
                item: 3213,
                one_in: 15,
                min: 1,
                max: 1,
            }],
        ],
        587 => &[
            &[Drop {
                item: 4608,
                one_in: 2,
                min: 4,
                max: 6,
            }],
            &[Drop {
                item: 3213,
                one_in: 15,
                min: 1,
                max: 1,
            }],
        ],
        590 => &[
            &[Drop {
                item: 8,
                one_in: 1,
                min: 5,
                max: 20,
            }],
            &[Drop {
                item: 216,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1304,
                one_in: 250,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5332,
                one_in: 1500,
                min: 1,
                max: 1,
            }],
        ],
        591 => &[
            &[Drop {
                item: 216,
                one_in: 50,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1304,
                one_in: 250,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 5332,
                one_in: 1500,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 8,
                one_in: 1,
                min: 5,
                max: 20,
            }],
        ],
        618 => &[
            &[Drop {
                item: 4608,
                one_in: 2,
                min: 7,
                max: 10,
            }],
            &[Drop {
                item: 4054,
                one_in: 10,
                min: 1,
                max: 1,
            }],
        ],
        620 => &[
            &[Drop {
                item: 4608,
                one_in: 2,
                min: 7,
                max: 10,
            }],
            &[Drop {
                item: 4270,
                one_in: 8,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 4317,
                one_in: 8,
                min: 1,
                max: 1,
            }],
        ],
        621 => &[
            &[Drop {
                item: 4608,
                one_in: 2,
                min: 7,
                max: 10,
            }],
            &[Drop {
                item: 4272,
                one_in: 8,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 4317,
                one_in: 8,
                min: 1,
                max: 1,
            }],
        ],
        628 => &[&[Drop {
            item: 313,
            one_in: 2,
            min: 1,
            max: 2,
        }]],
        630 => &[&[Drop {
            item: 527,
            one_in: 10,
            min: 1,
            max: 1,
        }]],
        631 => &[
            &[Drop {
                item: 3,
                one_in: 1,
                min: 10,
                max: 20,
            }],
            &[Drop {
                item: 4761,
                one_in: 3,
                min: 1,
                max: 1,
            }],
        ],
        634 => &[
            &[Drop {
                item: 4764,
                one_in: 40,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 18,
                one_in: 100,
                min: 1,
                max: 1,
            }],
        ],
        635 => &[&[
            Drop {
                item: 954,
                one_in: 100,
                min: 1,
                max: 1,
            },
            Drop {
                item: 955,
                one_in: 200,
                min: 1,
                max: 1,
            },
            Drop {
                item: 1166,
                one_in: 200,
                min: 1,
                max: 1,
            },
            Drop {
                item: 1274,
                one_in: 500,
                min: 1,
                max: 1,
            },
        ]],
        693 => &[
            &[Drop {
                item: 959,
                one_in: 450,
                min: 1,
                max: 1,
            }],
            &[Drop {
                item: 1307,
                one_in: 300,
                min: 1,
                max: 1,
            }],
        ],
        694 => &[&[Drop {
            item: 165,
            one_in: 40,
            min: 1,
            max: 1,
        }]],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The black lens sits behind a rarer drop, which is why it is not simply a one-in-three.
    #[test]
    fn a_demon_eye_rolls_its_rare_drop_before_its_lens() {
        let chain = drops(2)[0];
        assert!(chain.len() > 1, "it should be a chain");
        assert_eq!(chain.last().unwrap().item, 38, "the lens is the fallback");
        assert!(
            chain[0].one_in > chain.last().unwrap().one_in,
            "and the thing ahead of it is rarer"
        );
    }

    #[test]
    fn a_skeleton_has_several_weapons_behind_one_roll() {
        let chain = drops(21)[0];
        assert!(chain.len() >= 4, "got {chain:?}");
    }

    #[test]
    fn something_with_no_rules_drops_nothing() {
        assert!(drops(60_000).is_empty());
    }

    #[test]
    fn every_rule_is_a_sane_range() {
        for npc_type in 0..700u16 {
            for chain in drops(npc_type) {
                assert!(!chain.is_empty(), "type {npc_type} has an empty chain");
                for rule in *chain {
                    assert!(rule.one_in >= 1, "type {npc_type}");
                    assert!(rule.min >= 1 && rule.max >= rule.min, "type {npc_type}");
                }
            }
        }
    }

    #[test]
    fn a_good_many_of_the_roster_drop_something() {
        let with_loot = (0..700u16).filter(|t| !drops(*t).is_empty()).count();
        assert!(with_loot > 80, "only {with_loot} types have loot");
    }
}
