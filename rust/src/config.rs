use serde::Deserialize;

#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub struct Config {
    pub db_address: String,
    pub db_name: String,
    pub db_user: String,
    pub db_password: String,
}
