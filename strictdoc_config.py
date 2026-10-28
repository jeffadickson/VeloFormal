"""StrictDoc project configuration for VeloFormal requirements."""
from strictdoc.core.project_config import ProjectConfig


def create_config() -> ProjectConfig:
    """Return the project configuration used by StrictDoc export and validation."""
    return ProjectConfig(
        project_title="VeloFormal",
        project_features=["TRACEABILITY_SCREEN", "SEARCH"],
    )
