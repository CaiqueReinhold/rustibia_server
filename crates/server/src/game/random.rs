use rand::{RngExt, SeedableRng, rngs::Xoshiro256PlusPlus, seq::IndexedRandom};
use rand_distr::{Distribution, Normal};

use crate::constants::MAX_DROP_CHANCE;

pub struct Rolls {
    rng: Xoshiro256PlusPlus,
    normal: Normal<f32>,
}

impl Rolls {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Xoshiro256PlusPlus::seed_from_u64(seed),
            normal: Normal::new(0.5, 0.25).unwrap(),
        }
    }

    pub fn damage_roll(&mut self, min: u32, max: u32) -> u32 {
        let (a, b) = (min.min(max), min.max(max));
        let v = loop {
            let v = self.normal.sample(&mut self.rng);
            if (0.0..=1.0).contains(&v) {
                break v;
            }
        };
        a + (v * (b - a) as f32).round() as u32
    }

    pub fn uniform(&mut self, min: u32, max: u32) -> u32 {
        let (a, b) = (min.min(max), min.max(max));
        self.rng.random_range(a..=b)
    }

    pub fn category_roll<'a, T>(&mut self, choices: &'a [T]) -> Option<&'a T> {
        choices.choose(&mut self.rng)
    }

    // loot chance is expressed as 1/100000
    pub fn drop_chance(&mut self, chance: u32) -> bool {
        self.uniform(0, MAX_DROP_CHANCE - 1) < chance
    }

    // loot rate is expressed as integer percent value, e.g. 100 for regular 100% chance.
    pub fn drop_rolls(&mut self, loot_rate: u32) -> u32 {
        let scaled = loot_rate * MAX_DROP_CHANCE / 100;
        scaled / MAX_DROP_CHANCE
            + if self.uniform(0, MAX_DROP_CHANCE) < (scaled % MAX_DROP_CHANCE) {
                1
            } else {
                0
            }
    }
}
