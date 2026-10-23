# This document outlines the goals for this project, its invariants, etc. Its purpose is to be a somewhat modular composable game engine backend for my projects

1. This project is ideally going to be the game engine behind a REA port of COD: Black Ops 1 along with MattMC (my minecraft java port). both these games and this engine should use rust.
2. The project should be seperated and well organized into different crates.
3. Graphics should be handled by VulkanicGAL. VulkanicGAL Is originally copied from MattMC. both projects will continue using seperate implementations until this project is stable enoguh to support MattMC.
4. there should be higher level graphical abstractions available for games like mattmc and cod black ops to use.
5. all features of the engine should be drivable by code and CLI. A GUI SHOULD NOT be required to use the engine, to make games, etc.
6. an AI agent should be able to perform any and all actions a human can do in this project and to use this project.
7. a project should be able to use 1 module (say vulkanic) without requiring the person to use the entire engine for a project
8. a GUI should be available and actually useful for development, think godot, unity style editor. this gui AND the engine should not OWN the project, ie the project should be buildable and runnable without having to use an engine or gui specific run or test command.
9. it should support 2d and 3d games
10. Games should follow my https://github.com/HungLo2020/Basalt repo's STANDARDS.md format for games.
11. the engine GUI Editor itself should be built with QT and KDE style frameworks.
12. the engine should be cross platform. linux windows and macos. x86-64, arm64, and riscv64
13. VulkanicGAL should be a low level crate for supporting different rendering backends, Vulkan, OpenGL, Metal, DirectX etc. not a high level Renderer
14. Higher level rendering crates should handle high level functionality. HungLo2020/MattMC BlackOps Jackal MicroMages MissileCommand are all games i want to use Quasars rendering and engine functionality eventually
15. Using a higher level rendering crate should NOT prevent a game from using VulkanicGAL directly
16. Higher-level Quasar renderers and direct VulkanicGAL operations must be composable within the same graphics device, frame, and rendering pipeline. They must support explicit GPU resource sharing, synchronization, and compatible resource lifetime management without requiring redundant data transfers or separate graphics contexts.
