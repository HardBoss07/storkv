"""Build a platform-specific wheel for the StorKV Python binding."""

from setuptools import setup
from wheel.bdist_wheel import bdist_wheel


class BinaryWheel(bdist_wheel):
    """Mark the wheel as platform-specific because it carries a native library."""

    def finalize_options(self) -> None:
        super().finalize_options()
        self.root_is_pure = False


setup(cmdclass={"bdist_wheel": BinaryWheel})
