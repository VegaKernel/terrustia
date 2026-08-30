//! Faithful transcription of vanilla's array-based liquid simulator, water-only, kept **test-only**
//! as a measurement probe for the FIX-1c liquid crux.
//!
//! This is not the production simulator (`world/liquid.rs`), and it is never compiled into the
//! server. It exists to answer, with real numbers rather than a remembered claim, the one open
//! question the FIX-1c lane was created for: does vanilla's own levelling
//! (`Liquid.Update`'s flag2..flag7 `Math.Round` cases, `Liquid.cs:600-948`) satisfy strict
//! conservation and convergence when ported **as a whole unit** — that is, together with the
//! `kill` counter, the `DelWater` array-removal (`Liquid.cs:1481-1610`) and the `stuckCount`
//! force-settle (`Liquid.cs:1140-1158`) — the machinery FIX-1b's levelling-only attempt left out.
//!
//! ## The measured answer (the FIX-1c seam)
//!
//! Ported as a whole unit, the faithful mechanism **converges** and does **not** drain or thrash —
//! the `kill`/`DelWater`/`stuckCount` machinery FIX-1b lacked is exactly what stops both. Two
//! geometries, both settling to a perfectly level rest (`spread == 0`) with zero perpetual
//! oscillation (`faithful_port_converges_but_is_not_conservative` asserts this):
//! - an asymmetric pool (deep left, thin right): 30562 -> 30381, rest at tick 93, every column 779;
//! - a narrow odd-height pool built to provoke +/-1 thrash: 2212 -> **2214**, rest at tick 11.
//!
//! But it is **not conservative**, and that is the seam. Faithful `Math.Round` levelling
//! intrinsically creates and destroys small amounts of water: the thin-right pool ends `+2` units
//! (water created, `+0.09%`), the asymmetric one `-181` (thin-film evaporation, `-0.59%`). Creation
//! is the exact L3-12 water-duplication behaviour this project deliberately removed. So the faithful
//! port cannot satisfy the "no creation" half of the conservation criterion the production model
//! must meet — not because it drains or thrashes (it does neither), but because vanilla itself is
//! non-conservative here. The production `world/liquid.rs` keeps its simplified exact-division
//! levelling, which strictly conserves (never creates) and converges; this probe is the measured
//! evidence for that choice, and a regression guard against a future "just port vanilla" attempt.
//!
//! It transcribes the water path of vanilla verbatim:
//! - `Update` fall (`Liquid.cs:569-599`) including the `num == 1 && liquid == 255` +1 creation.
//! - `Update` level flag2..flag7 (`Liquid.cs:600-948`) with C# `Math.Round` (banker's rounding)
//!   and the `num2 = -1` thin-film term, and the `num3/num4` "already level" centre guard.
//! - `Update` end `kill` bookkeeping (`Liquid.cs:949-972`).
//! - `UpdateLiquid` round-robin + kill-sweep + `stuckCount` (`Liquid.cs:992-1159`).
//! - `AddWater` (`Liquid.cs:1169-1216`) and `DelWater` (`Liquid.cs:1481-1610`).
//!
//! Disclosed narrowings for the measurement (none affects the conservation/convergence question
//! for a bounded water pool): single liquid type (no lava/honey delay, no reactions — the
//! levelling math is identical), no `LiquidBuffer` and no panic mode (the buffer only fills, and
//! panic only arms, when `numLiquidBuffer` exceeds 90% of a 50000-entry buffer for 3600 ticks — a
//! bounded pool under `curMaxLiquid` never reaches that), and the `254 -> 255` random promotion
//! (`Liquid.cs:899`) is driven by a seeded RNG so the run is reproducible.

#![cfg(test)]

/// C# `Math.Round(double)`: round half to even ("banker's rounding"), the default `Update` uses.
fn round_half_even(v: f64) -> f64 {
    let floor = v.floor();
    let diff = v - floor;
    if diff < 0.5 {
        floor
    } else if diff > 0.5 {
        floor + 1.0
    } else if (floor as i64) % 2 == 0 {
        floor
    } else {
        floor + 1.0
    }
}

/// One entry of vanilla's `Main.liquid[]` array.
#[derive(Clone, Copy)]
struct Cell {
    x: i32,
    y: i32,
    kill: i32,
}

/// A tiny deterministic RNG so the `254 -> 255` promotion is reproducible in the measurement.
struct Rng(u64);
impl Rng {
    fn next_mod(&mut self, n: u64) -> u64 {
        // xorshift64*
        let mut z = self.0;
        z ^= z >> 12;
        z ^= z << 25;
        z ^= z >> 27;
        self.0 = z;
        (z.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) % n
    }
}

/// A faithful water-only liquid world plus vanilla's active-cell array.
struct Sim {
    w: i32,
    h: i32,
    liquid: Vec<u8>,
    solid: Vec<bool>,
    cells: Vec<Cell>,
    checking: Vec<bool>,
    skip: Vec<bool>,
    wet_counter: i32,
    cycles: i32,
    cur_max_liquid: i32,
    stuck_count: i32,
    stuck_amount: i32,
    stuck: bool,
    /// The kill threshold `num` in `UpdateLiquid`; 8 single-player, 10+players/3 on netMode 2.
    kill_threshold: i32,
    rng: Rng,
}

impl Sim {
    fn new(w: i32, h: i32) -> Self {
        let n = (w * h) as usize;
        Self {
            w,
            h,
            liquid: vec![0; n],
            solid: vec![false; n],
            cells: Vec::new(),
            checking: vec![false; n],
            skip: vec![false; n],
            wet_counter: 0,
            cycles: 10,
            cur_max_liquid: 25_000,
            stuck_count: 0,
            stuck_amount: 0,
            stuck: false,
            kill_threshold: 8,
            rng: Rng(0x1234_5678_9abc_def0),
        }
    }

    fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.w + x) as usize
    }
    fn l(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return 0;
        }
        self.liquid[self.idx(x, y)]
    }
    fn set_l(&mut self, x: i32, y: i32, v: u8) {
        let i = self.idx(x, y);
        self.liquid[i] = v;
    }
    /// A solid, non-platform tile — vanilla's `nactive() && tileSolid && !tileSolidTop`.
    fn solid_np(&self, x: i32, y: i32) -> bool {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return true;
        }
        self.solid[self.idx(x, y)]
    }

    fn total(&self) -> i64 {
        self.liquid.iter().map(|&v| i64::from(v)).sum()
    }
    fn num_liquid(&self) -> usize {
        self.cells.len()
    }

    /// `Liquid.AddWater` (`Liquid.cs:1169-1216`), water path.
    fn add_water(&mut self, x: i32, y: i32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let i = self.idx(x, y);
        if self.checking[i]
            || x >= self.w - 5
            || y >= self.h - 5
            || x < 5
            || y < 5
            || self.liquid[i] == 0
            || self.solid[i]
        {
            return;
        }
        if self.num_liquid() as i32 >= self.cur_max_liquid - 1 {
            // LiquidBuffer path — out of scope for the bounded measurement; drop rather than buffer.
            return;
        }
        self.checking[i] = true;
        self.skip[i] = false;
        self.cells.push(Cell { x, y, kill: 0 });
    }

    /// `Liquid.Update` (`Liquid.cs:451-973`), water path.
    fn update(&mut self, i: usize) {
        let (x, y) = (self.cells[i].x, self.cells[i].y);
        if self.solid_np(x, y) {
            self.cells[i].kill = 999;
            return;
        }
        let liquid_before = self.l(x, y);
        if liquid_before == 0 {
            self.cells[i].kill = 999;
            return;
        }

        // FALL (`Liquid.cs:569-599`). Water-only: below is same type or empty.
        let below = self.l(x, y + 1);
        if !self.solid_np(x, y + 1) && below < 255 {
            let t5 = self.l(x, y);
            let mut num = 255u16 - u16::from(below);
            if num > u16::from(t5) {
                num = u16::from(t5);
            }
            let flag = num == 1 && t5 == 255;
            if !flag {
                self.set_l(x, y, t5 - num as u8);
            }
            self.set_l(x, y + 1, below + num as u8);
            self.add_water(x, y + 1);
            let below_i = self.idx(x, y + 1);
            self.skip[below_i] = true;
            let here_i = self.idx(x, y);
            self.skip[here_i] = true;
            if !flag {
                self.add_water(x - 1, y);
                self.add_water(x + 1, y);
            }
        }

        // LEVEL (`Liquid.cs:600-948`).
        let t5 = self.l(x, y);
        if t5 > 0 {
            self.level(x, y, t5);
        }

        // KILL bookkeeping (`Liquid.cs:949-972`).
        let after = self.l(x, y);
        if after != liquid_before {
            if after == 254 && liquid_before == 255 {
                self.cells[i].kill += 1;
            } else {
                self.add_water(x, y - 1);
                self.cells[i].kill = 0;
            }
        } else {
            self.cells[i].kill += 1;
        }
    }

    /// The flag2..flag7 sideways levelling. `t5` is the current centre liquid.
    fn level(&mut self, x: i32, y: i32, t5: u8) {
        // flag2/flag3: immediate neighbours open. flag4/flag5: second neighbours have liquid.
        let flag2 = !self.solid_np(x - 1, y);
        let flag3 = !self.solid_np(x + 1, y);
        let mut flag4 = flag2 && !self.solid_np(x - 2, y) && self.l(x - 2, y) != 0;
        let mut flag5 = flag3 && !self.solid_np(x + 2, y) && self.l(x + 2, y) != 0;

        let num2: i32 = if t5 < 3 { -1 } else { 0 };
        if t5 > 250 {
            flag4 = false;
            flag5 = false;
        }

        if flag2 && flag3 {
            if flag4 && flag5 {
                let flag6 = !self.solid_np(x - 3, y) && self.l(x - 3, y) != 0;
                let flag7 = !self.solid_np(x + 3, y) && self.l(x + 3, y) != 0;
                if flag6 && flag7 {
                    self.level_seven(x, y, t5, num2);
                } else {
                    self.level_five(x, y, t5, num2);
                }
            } else if flag4 {
                self.level_four_left(x, y, t5, num2);
            } else if flag5 {
                self.level_four_right(x, y, t5, num2);
            } else {
                self.level_three(x, y, t5, num2);
            }
        } else if flag2 {
            self.level_two_left(x, y, t5, num2);
        } else if flag3 {
            self.level_two_right(x, y, t5, num2);
        }
    }

    /// 7-tile `Math.Round(sum/7)` case (`Liquid.cs:686-778`).
    fn level_seven(&mut self, x: i32, y: i32, t5: u8, num2: i32) {
        let sum = i32::from(self.l(x - 1, y))
            + i32::from(self.l(x + 1, y))
            + i32::from(self.l(x - 2, y))
            + i32::from(self.l(x + 2, y))
            + i32::from(self.l(x - 3, y))
            + i32::from(self.l(x + 3, y))
            + i32::from(t5)
            + num2;
        let num = round_half_even(f64::from(sum) / 7.0) as u8;
        let neighbours = [
            (x - 1, y),
            (x + 1, y),
            (x - 2, y),
            (x + 2, y),
            (x - 3, y),
            (x + 3, y),
        ];
        let mut num3 = 0;
        for (nx, ny) in neighbours {
            if self.l(nx, ny) != num {
                self.set_l(nx, ny, num);
                self.add_water(nx, ny);
            } else {
                num3 += 1;
            }
        }
        // Second-round AddWaters (`Liquid.cs:751-774`): `l(nb) != num || tile5.liquid != num`.
        for (nx, ny) in neighbours {
            if self.l(nx, ny) != num || t5 != num {
                self.add_water(nx, ny);
            }
        }
        if num3 != 6 || self.l(x, y - 1) == 0 {
            self.set_l(x, y, num);
        }
    }

    /// 5-tile `Math.Round(sum/5)` case (`Liquid.cs:782-844`).
    fn level_five(&mut self, x: i32, y: i32, t5: u8, num2: i32) {
        let sum = i32::from(self.l(x - 1, y))
            + i32::from(self.l(x + 1, y))
            + i32::from(self.l(x - 2, y))
            + i32::from(self.l(x + 2, y))
            + i32::from(t5)
            + num2;
        let num = round_half_even(f64::from(sum) / 5.0) as u8;
        let neighbours = [(x - 1, y), (x + 1, y), (x - 2, y), (x + 2, y)];
        let mut num4 = 0;
        for (nx, ny) in neighbours {
            if self.l(nx, ny) != num {
                self.set_l(nx, ny, num);
                self.add_water(nx, ny);
            } else {
                num4 += 1;
            }
        }
        for (nx, ny) in neighbours {
            if self.l(nx, ny) != num || t5 != num {
                self.add_water(nx, ny);
            }
        }
        if num4 != 4 || self.l(x, y - 1) == 0 {
            self.set_l(x, y, num);
        }
    }

    /// 4-tile left-asymmetric `Math.Round(sum/4)` case (`Liquid.cs:847-870`) — L3-11.
    fn level_four_left(&mut self, x: i32, y: i32, t5: u8, num2: i32) {
        let sum = i32::from(self.l(x - 1, y))
            + i32::from(self.l(x + 1, y))
            + i32::from(self.l(x - 2, y))
            + i32::from(t5)
            + num2;
        let num = round_half_even(f64::from(sum) / 4.0) as u8;
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x - 2, y)] {
            if self.l(nx, ny) != num || t5 != num {
                self.set_l(nx, ny, num);
                self.add_water(nx, ny);
            }
        }
        self.set_l(x, y, num);
    }

    /// 4-tile right-asymmetric `Math.Round(sum/4)` case (`Liquid.cs:871-894`) — L3-11.
    fn level_four_right(&mut self, x: i32, y: i32, t5: u8, num2: i32) {
        let sum = i32::from(self.l(x - 1, y))
            + i32::from(self.l(x + 1, y))
            + i32::from(self.l(x + 2, y))
            + i32::from(t5)
            + num2;
        let num = round_half_even(f64::from(sum) / 4.0) as u8;
        for (nx, ny) in [(x - 1, y), (x + 1, y), (x + 2, y)] {
            if self.l(nx, ny) != num || t5 != num {
                self.set_l(nx, ny, num);
                self.add_water(nx, ny);
            }
        }
        self.set_l(x, y, num);
    }

    /// 3-tile `Math.Round(sum/3)` case (`Liquid.cs:895-916`), with the seeded 254->255 promotion.
    fn level_three(&mut self, x: i32, y: i32, t5: u8, num2: i32) {
        let sum = i32::from(self.l(x - 1, y)) + i32::from(self.l(x + 1, y)) + i32::from(t5) + num2;
        let mut num = round_half_even(f64::from(sum) / 3.0);
        if num == 254.0 && self.rng.next_mod(30) == 0 {
            num = 255.0;
        }
        let num = num as u8;
        if self.l(x - 1, y) != num {
            self.set_l(x - 1, y, num);
            self.add_water(x - 1, y);
        }
        if self.l(x + 1, y) != num {
            self.set_l(x + 1, y, num);
            self.add_water(x + 1, y);
        }
        self.set_l(x, y, num);
    }

    /// 2-tile left `Math.Round(sum/2)` case (`Liquid.cs:918-932`).
    fn level_two_left(&mut self, x: i32, y: i32, t5: u8, num2: i32) {
        let sum = i32::from(self.l(x - 1, y)) + i32::from(t5) + num2;
        let num = round_half_even(f64::from(sum) / 2.0) as u8;
        if self.l(x - 1, y) != num {
            self.set_l(x - 1, y, num);
        }
        if t5 != num || self.l(x - 1, y) != num {
            self.add_water(x - 1, y);
        }
        self.set_l(x, y, num);
    }

    /// 2-tile right `Math.Round(sum/2)` case (`Liquid.cs:933-947`).
    fn level_two_right(&mut self, x: i32, y: i32, t5: u8, num2: i32) {
        let sum = i32::from(self.l(x + 1, y)) + i32::from(t5) + num2;
        let num = round_half_even(f64::from(sum) / 2.0) as u8;
        if self.l(x + 1, y) != num {
            self.set_l(x + 1, y, num);
        }
        if t5 != num || self.l(x + 1, y) != num {
            self.add_water(x + 1, y);
        }
        self.set_l(x, y, num);
    }

    /// `Liquid.DelWater` (`Liquid.cs:1481-1610`), water path. Returns whether the cell was removed
    /// from the array (vanilla's early `return` at 1519 keeps it, resetting kill).
    fn del_water(&mut self, l: usize) -> bool {
        let (x, y) = (self.cells[l].x, self.cells[l].y);
        let t1 = self.l(x - 1, y);
        let t2 = self.l(x + 1, y);
        let t3 = self.l(x, y + 1);
        let t4 = self.l(x, y);

        if t4 < 2 {
            self.set_l(x, y, 0);
            if t1 < 2 {
                self.set_l(x - 1, y, 0);
            } else {
                self.add_water(x - 1, y);
            }
            if t2 < 2 {
                self.set_l(x + 1, y, 0);
            } else {
                self.add_water(x + 1, y);
            }
        } else if t4 < 20 {
            if (t1 < t4 && !self.solid_np(x - 1, y))
                || (t2 < t4 && !self.solid_np(x + 1, y))
                || (t3 < 255 && !self.solid_np(x, y + 1))
            {
                self.set_l(x, y, 0);
            }
        } else if t3 < 255 && !self.solid_np(x, y + 1) && !self.stuck && !self.solid_np(x, y) {
            self.cells[l].kill = 0;
            return false;
        }

        let t4 = self.l(x, y);
        if t4 < 250 && self.l(x, y - 1) > 0 {
            self.add_water(x, y - 1);
        }
        if t4 != 0 {
            if t2 > 0 && t2 < 250 && !self.solid_np(x + 1, y) && t4 != t2 {
                self.add_water(x + 1, y);
            }
            if t1 > 0 && t1 < 250 && !self.solid_np(x - 1, y) && t4 != t1 {
                self.add_water(x - 1, y);
            }
        }

        // numLiquid--; checking(false); swap last into l (`Liquid.cs:1586-1590`).
        let i = self.idx(x, y);
        self.checking[i] = false;
        self.cells.swap_remove(l);
        true
    }

    /// `Liquid.UpdateLiquid` (`Liquid.cs:992-1159`), the bounded-pool path (no panic, no buffer).
    fn update_liquid(&mut self) {
        let num = self.kill_threshold;
        self.wet_counter += 1;
        let num4 = self.cur_max_liquid / self.cycles;
        let num5 = num4 * (self.wet_counter - 1);
        let mut num6 = num4 * self.wet_counter;
        if self.wet_counter == self.cycles {
            num6 = self.num_liquid() as i32;
        }
        if num6 > self.num_liquid() as i32 {
            num6 = self.num_liquid() as i32;
            self.wet_counter = self.cycles;
        }
        for n in num5..num6 {
            let n = n as usize;
            if n >= self.cells.len() {
                break;
            }
            let (cx, cy) = (self.cells[n].x, self.cells[n].y);
            let ci = self.idx(cx, cy);
            if !self.skip[ci] {
                self.update(n);
            } else {
                self.skip[ci] = false;
            }
        }

        if self.wet_counter >= self.cycles {
            self.wet_counter = 0;
            // kill-sweep (`Liquid.cs:1118-1128`), reverse order.
            let start = self.num_liquid();
            for n7 in (0..start).rev() {
                if n7 >= self.cells.len() {
                    continue;
                }
                if self.cells[n7].kill >= num {
                    let (cx, cy) = (self.cells[n7].x, self.cells[n7].y);
                    if self.l(cx, cy) == 254 {
                        self.set_l(cx, cy, 255);
                    }
                    self.del_water(n7);
                }
            }
            // stuckCount (`Liquid.cs:1140-1158`).
            let n = self.num_liquid() as i32;
            if n > 0 && n > self.stuck_amount - 50 && n < self.stuck_amount + 50 {
                self.stuck_count += 1;
                if self.stuck_count >= 10_000 {
                    self.stuck = true;
                    for n10 in (0..self.num_liquid()).rev() {
                        if n10 < self.cells.len() {
                            self.del_water(n10);
                        }
                    }
                    self.stuck = false;
                    self.stuck_count = 0;
                }
            } else {
                self.stuck_count = 0;
                self.stuck_amount = n;
            }
        }
    }

    /// Pour liquid straight into a tile (a bucket or a mined block), without registering the cell.
    fn pour(&mut self, x: i32, y: i32, amount: u8) {
        self.set_l(x, y, amount);
    }
    fn wall(&mut self, x: i32, y: i32) {
        let i = self.idx(x, y);
        self.solid[i] = true;
    }
    /// Total liquid held in one column, across all its rows.
    fn column(&self, x: i32) -> i64 {
        (0..self.h).map(|y| i64::from(self.l(x, y))).sum()
    }
}

#[cfg(test)]
mod probe {
    use super::*;

    /// Build a boxed asymmetric pool: a floor, side walls, and an uneven surface, then wake every
    /// wet tile through AddWater exactly as a disturbance would.
    fn asymmetric_pool() -> Sim {
        let (w, h) = (60, 40);
        let mut sim = Sim::new(w, h);
        // Floor at y = 30, walls at x = 10 and x = 50 (both clear of the 5-tile border margin).
        for x in 0..w {
            sim.wall(x, 30);
        }
        for y in 0..=30 {
            sim.wall(10, y);
            sim.wall(50, y);
        }
        // An asymmetric fill: a deep stack on the left, a thin sheet on the right, with a partial
        // top tile so the totals do not divide evenly.
        for x in 11..50 {
            let depth = if x < 20 {
                6
            } else if x < 30 {
                3
            } else {
                1
            };
            for d in 0..depth {
                sim.pour(x, 30 - 1 - d, 255);
            }
            sim.pour(x, 30 - 1 - depth, (x as u8).wrapping_mul(7) % 200 + 20);
        }
        for x in 11..50 {
            for y in 20..30 {
                if sim.l(x, y) > 0 {
                    sim.add_water(x, y);
                }
            }
        }
        sim
    }

    /// A narrow, deep, odd-total pool between two close walls — the geometry most prone to vanilla's
    /// +/-1 boundary thrash, since 2- and 3-tile `Math.Round` cases dominate and the totals never
    /// divide evenly.
    fn thrash_prone_pool() -> Sim {
        let (w, h) = (24, 30);
        let mut sim = Sim::new(w, h);
        for x in 0..w {
            sim.wall(x, 22);
        }
        for y in 0..=22 {
            sim.wall(8, y);
            sim.wall(15, y);
        }
        // Six columns (x=9..14), each a different odd-ish height, so no flat rest divides evenly.
        for (x, top) in [(9, 253), (10, 191), (11, 127), (12, 63), (13, 31), (14, 17)] {
            sim.pour(x, 21, 255);
            sim.pour(x, 20, top);
        }
        for x in 9..15 {
            for y in 15..22 {
                if sim.l(x, y) > 0 {
                    sim.add_water(x, y);
                }
            }
        }
        sim
    }

    /// What one settled run reports.
    struct Report {
        before: i64,
        after: i64,
        rest_tick: Option<usize>,
        max_late_swing: i64,
        spread: i64,
    }

    /// Run a pool to rest and report conservation, convergence, thrash and final surface flatness.
    fn run_and_report(mut sim: Sim, label: &str, lo: i32, hi: i32) -> Report {
        let before = sim.total();
        let mut rest_tick: Option<usize> = None;
        let mut max_late_swing: i64 = 0;
        let n_ticks = 20_000usize;
        for t in 0..n_ticks {
            let pre = sim.total();
            sim.update_liquid();
            let post = sim.total();
            if sim.num_liquid() == 0 && rest_tick.is_none() {
                rest_tick = Some(t + 1);
            }
            if t > 2_000 {
                max_late_swing = max_late_swing.max((post - pre).abs());
            }
        }
        let after = sim.total();
        let cols: Vec<i64> = (lo..hi).map(|x| sim.column(x)).collect();
        let cmin = cols.iter().copied().min().unwrap_or(0);
        let cmax = cols.iter().copied().max().unwrap_or(0);
        let spread = cmax - cmin;
        // Printed under `--nocapture`; the assertions below are the durable record.
        println!(
            "{label}: conservation {before} -> {after} ({:+} units, {:.2}%); rest at tick {rest_tick:?}; \
             late swing {max_late_swing}; final columns {lo}..{hi} = {cols:?} (spread {spread})",
            after - before,
            (after - before) as f64 / before as f64 * 100.0
        );
        Report {
            before,
            after,
            rest_tick,
            max_late_swing,
            spread,
        }
    }

    /// The FIX-1c liquid crux, as a measured, asserting record. Demonstrates that the faithful
    /// vanilla mechanism, ported as a whole unit (levelling + `kill` + `DelWater` + `stuckCount`):
    ///
    /// 1. CONVERGES: both an asymmetric pool and a narrow pool built to provoke +/-1 thrash reach a
    ///    perfectly level rest (`spread == 0`) in well under a hundred ticks, with zero late
    ///    oscillation. The `kill`/`DelWater`/`stuckCount` machinery FIX-1b's levelling-only attempt
    ///    lacked is exactly what stops the drain-to-zero and the perpetual thrash it measured.
    /// 2. Is NOT CONSERVATIVE: faithful `Math.Round` levelling *creates* water on the thrash-prone
    ///    pool (`after > before`, measured `+2`), the same L3-12 duplication the production model
    ///    removed. So it cannot meet the "no creation" conservation criterion, which is the seam:
    ///    the production `world/liquid.rs` keeps exact-division levelling for that reason.
    ///
    /// See the module doc for the full seam write-up. Run verbose with:
    /// `cargo test -p terrustia --lib world::liquid_faithful -- --nocapture`
    #[test]
    fn faithful_port_converges_but_is_not_conservative() {
        // Asymmetric pool: deep left, thin right, columns x=11..49 between walls x=10/50.
        let a = run_and_report(asymmetric_pool(), "asymmetric pool", 11, 50);
        // Narrow odd-height pool built to provoke +/-1 thrash: columns x=9..14 between walls x=8/15.
        let b = run_and_report(thrash_prone_pool(), "thrash-prone pool", 9, 15);

        // (1) CONVERGENCE: both settle to a perfectly level rest, no perpetual thrash.
        for r in [&a, &b] {
            assert!(r.rest_tick.is_some(), "did not reach rest (thrash/drain)");
            assert!(
                r.rest_tick.unwrap() < 500,
                "took too long to settle: {:?}",
                r.rest_tick
            );
            assert_eq!(r.spread, 0, "did not settle level: spread {}", r.spread);
            assert_eq!(
                r.max_late_swing, 0,
                "still oscillating late: swing {}",
                r.max_late_swing
            );
        }

        // (2) NON-CONSERVATION is the seam. It never drains to zero and never blows up (both stay
        // within 1% of their start), but the thrash-prone pool ends with MORE water than it began —
        // faithful vanilla created it. That is why the production model does not use this levelling.
        for r in [&a, &b] {
            assert!(
                r.after > r.before * 99 / 100,
                "drained: {} -> {}",
                r.before,
                r.after
            );
            assert!(
                r.after < r.before * 101 / 100,
                "blew up: {} -> {}",
                r.before,
                r.after
            );
        }
        assert!(
            b.after > b.before,
            "faithful Math.Round levelling should CREATE water on the thrash-prone pool \
             (measured +2), which is the L3-12 duplication that fails strict conservation: {} -> {}",
            b.before,
            b.after
        );
    }
}
