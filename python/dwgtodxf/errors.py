class DwgError(Exception):
    """Base class for dwgtodxf errors (unreadable, corrupt or unsupported DWG)."""


class UnsupportedVersionError(DwgError):
    """The DWG version is not supported (pre-R13, or not implemented yet)."""
