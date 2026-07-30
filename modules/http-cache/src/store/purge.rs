use http::{header, HeaderMap};

use crate::lscache::{PurgeOperation, PurgeSelector};
use crate::policy::CacheScope;

use super::types::StoredEntry;

pub(crate) fn entry_matches_purge(
    entry: &StoredEntry,
    operation: &PurgeOperation,
    current_private_key: Option<&str>,
) -> bool {
    if entry.scope != operation.scope {
        return false;
    }

    if operation.scope == CacheScope::Private
        && current_private_key.is_some()
        && entry.private_key.as_deref() != current_private_key
    {
        return false;
    }

    operation.selectors.iter().any(|selector| match selector {
        PurgeSelector::All => true,
        PurgeSelector::Url(url) => entry.purge_url == *url,
        PurgeSelector::UrlPath(path) => {
            let normalized_purge_url = entry
                .purge_url
                .split_once(['?', '#'])
                .map_or(entry.purge_url.as_str(), |(url, _)| url);
            normalized_purge_url == *path
        }
        PurgeSelector::Tag(tag) => entry
            .tags
            .iter()
            .any(|entry_tag| entry_tag.scope == operation.scope && entry_tag.name == *tag),
    })
}

pub fn strip_store_headers(headers: &mut HeaderMap) {
    headers.remove(header::AGE);
}
