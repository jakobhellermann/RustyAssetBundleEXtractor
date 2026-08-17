// I want to test with realistic data, but don't want to commit copyrighted material or
// develop that exhaustive test suite myself.
pub const EXTERNAL_GAME_PATHS: &[&str] = &[
    // "/home/jakob/.local/share/Steam/steamapps/common/Hollow Knight/hollow_knight_Data",
    // runs into problems with float infinity as null in serde_json
    // "/home/jakob/.local/share/Steam/steamapps/common/Nine Sols-Speedrunpatch/NineSols_Data",
    // Unity 5.4.3f1, serialized format v15
    //"/path/to/Hollow Knight-1.2.2.1/hollow_knight.app/Contents/Resources/Data",
];
