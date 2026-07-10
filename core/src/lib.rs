//! bpsr-core: パケット観測・TCP再組み立て・プロトコル解析・DPS集計のコア。
//! UI フレームワーク（Tauri/Slint）に依存しない。フロントはこのクレートの
//! `compute` 関数群を直接呼び、`EncounterMutex` を共有して集計結果を取得する。

pub mod capture;
pub mod compute;
pub mod engine;
pub mod error;
pub mod models;
pub mod probe;
pub mod protocol;
