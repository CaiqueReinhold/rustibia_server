#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CombatElement {
    Physical,
    Energy,
    Fire,
    Earth,
    Ice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponType {
    None,
    Axe,
    Club,
    Sword,
    Bow,
    Crossbow,
    Distance,
    Wand,
    Rod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AmmoType {
    Arrow,
    Bolt,
}

#[derive(Debug, Clone)]
pub struct CombatDamage {
    pub element: CombatElement,
    pub value: u32,
    pub blocked_shield: bool,
    pub blocked_armor: bool,
}
