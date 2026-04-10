import os

import pytest


@pytest.fixture(scope="session")
def server_url():
    url = os.environ.get("FS123_TEST_SERVER_URL")
    if not url:
        pytest.skip("FS123_TEST_SERVER_URL not set — run via scripts/run_integration_tests.py")
    return url


@pytest.fixture(scope="session")
def mountpoint():
    return os.environ.get("FS123_TEST_MOUNTPOINT", "/fs123_test")


@pytest.fixture(scope="session", autouse=True)
def mount_server(server_url, mountpoint):
    """Mount once for the entire test session, unmount at end."""
    import fs123

    fs123.set_proto("7.3")
    fs123.mount(server_url, mountpoint)
    yield mountpoint
    fs123.umount(mountpoint)
