error[E0277]: `std::result::Result<AsyncStorage, tsink::TsinkError>` is not a future
  --> crates/daly-bms-server/src/tsink_db.rs:69:14
   |
69 |             .await?;
   |              ^^^^^ `std::result::Result<AsyncStorage, tsink::TsinkError>` is not a future
   |
   = help: the trait `futures::Future` is not implemented for `std::result::Result<AsyncStorage, tsink::TsinkError>`
   = note: std::result::Result<AsyncStorage, tsink::TsinkError> must be a future or must implement `IntoFuture` to be awaited
   = note: required for `std::result::Result<AsyncStorage, tsink::TsinkError>` to implement `std::future::IntoFuture`
help: remove the `.await`
   |
69 -             .await?;
69 +             ?;
   |


