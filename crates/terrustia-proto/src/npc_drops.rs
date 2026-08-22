//! What NPCs drop when they die, transcribed from `ItemDropDatabase`.
//!
//! The game's loot system is a tree of rules. Most of it is one shape: roll a one-in-N chance, and
//! if it misses, roll the next thing instead. A skeleton's four possible weapons are one such
//! chain, and a demon eye's black lens sits behind its rarer drop the same way. That structure is
//! preserved here — a chain is tried in order and stops at the first success — because collapsing
//! it into independent rolls would make several drops far more common than they are.
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
        6 => &[&[Drop {
            item: 1309,
            one_in: 10000,
            min: 1,
            max: 1,
        }]],
        7 => &[&[Drop {
            item: 1309,
            one_in: 10000,
            min: 1,
            max: 1,
        }]],
        16 => &[&[Drop {
            item: 1309,
            one_in: 10000,
            min: 1,
            max: 1,
        }]],
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
        48 => &[&[Drop {
            item: 1516,
            one_in: 150,
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
        104 => &[&[Drop {
            item: 485,
            one_in: 60,
            min: 1,
            max: 1,
        }]],
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
        167 => &[
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
        185 => &[
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
        513 => &[&[Drop {
            item: 3380,
            one_in: 2,
            min: 1,
            max: 2,
        }]],
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
        550 => &[&[Drop {
            item: 3821,
            one_in: 8,
            min: 1,
            max: 1,
        }]],
        590 => &[
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
        628 => &[&[Drop {
            item: 313,
            one_in: 2,
            min: 1,
            max: 2,
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
