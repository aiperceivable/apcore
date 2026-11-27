"""Full runtime assembly for applications using configuration and discovery."""

from apcore.acl import ACL
from apcore.client import APCore
from apcore.config import Config
from apcore.discovery import Registry

__all__ = ["ACL", "APCore", "Config", "Registry"]
