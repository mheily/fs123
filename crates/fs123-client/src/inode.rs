//! Inode management for fs123 FUSE filesystem.
//!
//! fs123 is a path-based protocol, but FUSE requires inode-based operations.
//! This module provides a bidirectional mapping between paths and inodes,
//! as well as tracking of ESTALE cookies for consistency.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;

/// Root inode number (always 1 in FUSE).
pub const ROOT_INO: u64 = 1;

/// Information stored for each inode.
#[derive(Debug, Clone)]
pub struct InodeInfo {
    /// Full path from root
    pub path: String,
    /// ESTALE cookie from server (for consistency checking)
    pub estalecookie: u64,
    /// Reference count (number of lookups minus forgets)
    pub refcount: u64,
    /// Last known file type (for quick directory detection)
    #[allow(unused)]
    pub file_type: u8,
}

/// Manages the mapping between inodes and paths.
pub struct InodeManager {
    /// Next inode number to allocate
    next_ino: AtomicU64,
    /// Map from inode to info
    ino_to_info: RwLock<HashMap<u64, InodeInfo>>,
    /// Map from path to inode
    path_to_ino: RwLock<HashMap<String, u64>>,
}

impl InodeManager {
    /// Create a new inode manager with root inode pre-allocated.
    pub fn new() -> Self {
        let manager = InodeManager {
            next_ino: AtomicU64::new(ROOT_INO + 1),
            ino_to_info: RwLock::new(HashMap::new()),
            path_to_ino: RwLock::new(HashMap::new()),
        };

        // Pre-allocate root inode
        {
            let mut ino_to_info = manager.ino_to_info.write().unwrap();
            let mut path_to_ino = manager.path_to_ino.write().unwrap();

            ino_to_info.insert(
                ROOT_INO,
                InodeInfo {
                    path: "/".to_string(),
                    estalecookie: 0,
                    refcount: 1, // Root always has at least 1 ref
                    file_type: libc::DT_DIR as u8,
                },
            );
            path_to_ino.insert("/".to_string(), ROOT_INO);
        }

        manager
    }

    /// Get the path for an inode.
    pub fn get_path(&self, ino: u64) -> Option<String> {
        let ino_to_info = self.ino_to_info.read().unwrap();
        ino_to_info.get(&ino).map(|info| info.path.clone())
    }

    /// Get the inode for a path, if it exists.
    #[cfg(test)]
    pub fn get_ino(&self, path: &str) -> Option<u64> {
        let path_to_ino = self.path_to_ino.read().unwrap();
        path_to_ino.get(path).copied()
    }

    /// Get or create an inode for a path.
    ///
    /// If the path already has an inode with a matching estalecookie, returns it.
    /// If the estalecookie differs, invalidates the old inode and creates a new one.
    /// If the path is new, creates a new inode.
    pub fn get_or_create(&self, path: &str, estalecookie: u64, file_type: u8) -> u64 {
        // Check if we already have this path
        {
            let path_to_ino = self.path_to_ino.read().unwrap();
            if let Some(&ino) = path_to_ino.get(path) {
                let ino_to_info = self.ino_to_info.read().unwrap();
                if let Some(info) = ino_to_info.get(&ino) {
                    // If estalecookie is 0 or matches, reuse the inode
                    if estalecookie == 0 || info.estalecookie == 0 || info.estalecookie == estalecookie
                    {
                        return ino;
                    }
                    // estalecookie mismatch - need to create a new inode
                    // (the old one will be cleaned up via forget)
                }
            }
        }

        // Allocate new inode
        let ino = self.next_ino.fetch_add(1, Ordering::SeqCst);

        let mut ino_to_info = self.ino_to_info.write().unwrap();
        let mut path_to_ino = self.path_to_ino.write().unwrap();

        ino_to_info.insert(
            ino,
            InodeInfo {
                path: path.to_string(),
                estalecookie,
                refcount: 1,
                file_type,
            },
        );
        path_to_ino.insert(path.to_string(), ino);

        ino
    }

    /// Increment the reference count for an inode.
    pub fn lookup(&self, ino: u64) {
        let mut ino_to_info = self.ino_to_info.write().unwrap();
        if let Some(info) = ino_to_info.get_mut(&ino) {
            info.refcount += 1;
        }
    }

    /// Decrement the reference count for an inode.
    /// If the refcount reaches 0 and the inode isn't root, it can be cleaned up.
    pub fn forget(&self, ino: u64, nlookup: u64) {
        if ino == ROOT_INO {
            return; // Never forget root
        }

        let mut ino_to_info = self.ino_to_info.write().unwrap();
        let mut path_to_ino = self.path_to_ino.write().unwrap();

        if let Some(info) = ino_to_info.get_mut(&ino) {
            info.refcount = info.refcount.saturating_sub(nlookup);
            if info.refcount == 0 {
                path_to_ino.remove(&info.path);
                ino_to_info.remove(&ino);
            }
        }
    }

    /// Update the estalecookie for an inode.
    pub fn update_estalecookie(&self, ino: u64, estalecookie: u64) {
        let mut ino_to_info = self.ino_to_info.write().unwrap();
        if let Some(info) = ino_to_info.get_mut(&ino) {
            info.estalecookie = estalecookie;
        }
    }

    /// Get the estalecookie for an inode.
    pub fn get_estalecookie(&self, ino: u64) -> Option<u64> {
        let ino_to_info = self.ino_to_info.read().unwrap();
        ino_to_info.get(&ino).map(|info| info.estalecookie)
    }

    /// Get the file type for an inode.
    // #[cfg(test)]
    // pub fn get_file_type(&self, ino: u64) -> Option<u8> {
    //     let ino_to_info = self.ino_to_info.read().unwrap();
    //     ino_to_info.get(&ino).map(|info| info.file_type)
    // }

    /// Build a child path from parent inode and name.
    pub fn child_path(&self, parent_ino: u64, name: &str) -> Option<String> {
        let parent_path = self.get_path(parent_ino)?;
        if parent_path == "/" {
            Some(format!("/{}", name))
        } else {
            Some(format!("{}/{}", parent_path, name))
        }
    }
}

impl Default for InodeManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_inode() {
        let manager = InodeManager::new();
        assert_eq!(manager.get_path(ROOT_INO), Some("/".to_string()));
        assert_eq!(manager.get_ino("/"), Some(ROOT_INO));
    }

    #[test]
    fn test_get_or_create() {
        let manager = InodeManager::new();

        let ino1 = manager.get_or_create("/foo", 12345, libc::DT_REG as u8);
        assert!(ino1 > ROOT_INO);

        // Same path and cookie should return same inode
        let ino2 = manager.get_or_create("/foo", 12345, libc::DT_REG as u8);
        assert_eq!(ino1, ino2);

        // Different cookie should return new inode
        let ino3 = manager.get_or_create("/foo", 99999, libc::DT_REG as u8);
        assert_ne!(ino1, ino3);
    }

    #[test]
    fn test_child_path() {
        let manager = InodeManager::new();

        assert_eq!(manager.child_path(ROOT_INO, "foo"), Some("/foo".to_string()));

        let ino = manager.get_or_create("/bar", 0, libc::DT_DIR as u8);
        assert_eq!(
            manager.child_path(ino, "baz"),
            Some("/bar/baz".to_string())
        );
    }

    #[test]
    fn test_forget() {
        let manager = InodeManager::new();

        let ino = manager.get_or_create("/test", 123, libc::DT_REG as u8);
        assert!(manager.get_path(ino).is_some());

        // Forget should remove it when refcount reaches 0
        manager.forget(ino, 1);
        assert!(manager.get_path(ino).is_none());
    }
}
