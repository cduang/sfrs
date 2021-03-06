use crate::schema::items;
use crate::schema::items::dsl::*;
use crate::{SqliteLike, lock_db_write, lock_db_read};
use crate::user;
use diesel::dsl::max;
use diesel::prelude::*;
use serde::{Serialize, Deserialize};
use std::vec::Vec;

#[derive(Debug)]
pub struct ItemOpError(pub String);

impl ItemOpError {
    fn new(s: impl Into<String>) -> ItemOpError {
        ItemOpError(s.into())
    }
}

impl Into<ItemOpError> for &str {
    fn into(self) -> ItemOpError {
        ItemOpError::new(self)
    }
}

#[derive(Queryable)]
pub struct Item {
    // This "id", though primary key, is not how the client actually
    // identifies an item, and it is not sent to the client.
    // Instead, this "id" is more like a "timestamp", in the sense
    // that each time an item is modified, it increments.
    // (this incrementing is achieved by deleting and re-inserting
    //  the item and relying on AUTOINCREMENT)
    // This is used in place of the role of timestamp in the Ruby
    // and Go implementation.
    pub id: i64,
    pub owner: i32,
    pub uuid: String,
    pub content: Option<String>,
    pub content_type: String,
    pub enc_item_key: Option<String>,
    pub deleted: bool,
    pub created_at: String,
    pub updated_at: Option<String>
}

#[derive(Insertable)]
#[table_name = "items"]
struct InsertItem {
    owner: i32,
    uuid: String,
    content: Option<String>,
    content_type: String,
    enc_item_key: Option<String>,
    deleted: bool,
    created_at: String,
    updated_at: Option<String>
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SyncItem {
    pub uuid: String,
    pub content: Option<String>,
    pub content_type: String,
    pub enc_item_key: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    pub created_at: String,
    pub updated_at: Option<String>
}

impl Into<SyncItem> for Item {
    fn into(self) -> SyncItem {
        SyncItem {
            uuid: self.uuid,
            content: self.content,
            content_type: self.content_type,
            enc_item_key: self.enc_item_key,
            deleted: self.deleted,
            created_at: self.created_at,
            updated_at: self.updated_at
        }
    }
}

impl SyncItem {
    pub fn items_of_user(
        db: &impl SqliteLike, u: &user::User,
        since_id: Option<i64>, max_id: Option<i64>,
        limit: Option<i64>
    ) -> Result<Vec<Item>, ItemOpError> {
        lock_db_read!()
            .and_then(|_| {
                let mut stmt = items.filter(owner.eq(u.id)).into_boxed();
                if let Some(limit) = limit {
                    stmt = stmt.limit(limit);
                }

                if let Some(since_id) = since_id {
                    stmt = stmt.filter(id.gt(since_id));
                }

                if let Some(max_id) = max_id {
                    stmt = stmt.filter(id.le(max_id));
                }

                stmt.order(id.asc())
                    .load::<Item>(db)
                    .map_err(|_| "Database error".into())
            })
    }

    pub fn find_item_by_uuid(db: &impl SqliteLike, u: &user::User, i: &str) -> Result<Item, ItemOpError> {
        lock_db_read!()
            .and_then(|_| {
                items.filter(owner.eq(u.id).and(uuid.eq(i)))
                    .first::<Item>(db)
                    .map_err(|_| "Database error".into())
            })
    }

    // Get the current maximum item ID for a user.
    // Remember that IDs do not identify item; instead, they are incremented to the largest value
    // every time an item is updated (see Self::items_insert).
    // The ID returned by this function is more like a "timestamp" of the latest "state"
    pub fn get_current_max_id(db: &impl SqliteLike, u: &user::User) -> Result<Option<i64>, ItemOpError> {
        lock_db_read!()
            .and_then(|_| {
                items.filter(owner.eq(u.id))
                    .select(max(id))
                    .first::<Option<i64>>(db)
                    .map_err(|_| "Database error".into())
            })
    }

    pub fn items_insert(db: &impl SqliteLike, u: &user::User, it: &SyncItem) -> Result<i64, ItemOpError> {
        // First, try to find the original item, if any, delete it, and insert a new one with the same UUID
        // This way, the ID is updated each time an item is updated
        // This method acts both as insertion and update
        let orig = lock_db_read!()
            .and_then(|_| {
                items.filter(uuid.eq(&it.uuid).and(owner.eq(u.id)))
                    .load::<Item>(db)
                    .map_err(|_| "Database error".into())
            })?;

        let _lock = lock_db_write!()?;
        if !orig.is_empty() {
            diesel::delete(items.filter(uuid.eq(&it.uuid).and(owner.eq(u.id))))
                .execute(db)
                .map(|_| ())
                .map_err(|_| "Database error".into())?;
        }

        diesel::insert_into(items::table)
            .values(InsertItem {
                owner: u.id,
                uuid: it.uuid.clone(),
                content: if it.deleted { None } else { it.content.clone() },
                content_type: it.content_type.clone(),
                enc_item_key: if it.deleted { None } else { it.enc_item_key.clone() },
                deleted: it.deleted,
                created_at: it.created_at.clone(),
                updated_at: it.updated_at.clone()
            })
            .execute(db)
            .map_err(|_| "Database error".into())?;
        std::mem::drop(_lock);

        Self::find_item_by_uuid(db, u, &it.uuid)
            .map(|i| i.id)
    }
}