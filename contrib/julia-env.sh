# Julia + jlrs environment in POSIX shell syntax (README, "Environment file").
#
# One file shared by both shells:
#   bash:  source ~/.config/julia-env.sh        (from ~/.bashrc)
#   fish:  bass source ~/.config/julia-env.sh   (from config.fish)
#
# Install:  cp contrib/julia-env.sh ~/.config/julia-env.sh

JULIA_HOME="$HOME/julia"   # symlink: ln -sfn ~/julia-1.13.4 ~/julia

export JLRS_JULIA_DIR="$JULIA_HOME"
export PATH="$JULIA_HOME/bin:$PATH"
export LD_LIBRARY_PATH="$JULIA_HOME/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
