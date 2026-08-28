use rand::{RngExt, SeedableRng, rngs::Xoshiro256PlusPlus, seq::IndexedRandom};
use rand_distr::{Distribution, Normal};

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
}
