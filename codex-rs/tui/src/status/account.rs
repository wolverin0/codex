#[derive(Debug, Clone)]
pub(crate) enum StatusAccountDisplay {
    ChatGpt {
        email: Option<String>,
        plan: Option<String>,
        account_display_name: Option<String>,
        account_group_names: Option<Vec<String>>,
    },
    ApiKey,
}
