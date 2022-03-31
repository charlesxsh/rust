use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Eq, PartialEq, Debug)]
pub enum UnsafeReason {
    // unsafe because of source code' unsafe block
    Original,

    // unsafe 
    UnsafeWrite,
    ParentHasUnsafeField,
    UnsafeParam
}



#[derive(Deserialize, Serialize, Eq, PartialEq, Debug)]
pub struct UnsafeRegion {
    pub file: String,
    pub line: u32,
    pub col_left: u32,
    pub col_right: u32,
    pub reason: UnsafeReason
}

impl ToString for UnsafeRegion {
    fn to_string(&self) -> String {
        format!("{}:{} {}-{} {:?}", self.file, self.line, self.col_left, self.col_right, self.reason)
    }
}