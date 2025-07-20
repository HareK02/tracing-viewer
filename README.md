# Tracing Viewer

`tracing-viewer`は、Rustの`tracing`クレートから出力されたログをインタラクティブにフィルタリングし、表示するためのTUIツールです。

## 機能

*   ファイルまたは標準入力から`tracing`ログを読み込み
*   モジュール単位でのログの表示/非表示フィルタリング
*   ファイル監視によるログのリアルタイム更新
*   選択したログのクリップボードへのコピー

## Installation

1. git clone
2. cargo build --release
3. cargo install --path .