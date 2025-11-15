# Open Football

Attempt to implement Sigames Football Manager simulation engine without manual control.

The project is NOT a game, it is a simulation without the possibility of control.

The goal is to get as close as possible to a real soccer simulation and based on this data:

- Predict match results
- Predict the success of future player transfers
- Simulate other football data

Currently available nation for simulation - **Italy**

---
**[Live Demo](https://open-football.org)**

*The simulation is quite resource-intensive, so I disabled the continue button (top-rigth corner) after first simulation*

---


[Match example (click on any goals)](https://open-football.org/leagues/italian-serie-a)

![alt text](.docs/images/match.avif "Match page")

[Player page example (click on any player)](https://open-football.org/teams/juventus)
![alt text](.docs/images/player.jpg "Player page")

[Club page example](https://open-football.org/teams/juventus)

![alt text](.docs/images/club.jpg "Club page")

[League page example](https://open-football.org/leagues/italian-serie-a)

![alt text](.docs/images/league.jpg "League page")

#### Project structure

/src/core - Core Rust app logic (including match)

/src/database - Simulation data source logic

/src/dev/graphics - Dev utils for instant match development (src/core/src/match)

Match dev looks like (cross-platform)

![alt text](.docs/images/match_dev.png "Match dev tools")

/src/dev/neural - Dev utils for training NN for using in match, etc

/src/neural - Core Burn neural network data
/src/server - HTTP server for running API for Angular UI

/ui - Angular app that you can see in **[Live Demo](https://open-football.org)**

---

#### How to run?

1) Local run

```console
// run frontend (Angular)
cd ui
npm install --force
npm start
...
// run backend
cargo run
...
open chrome at http://localhost:18000
```

2) Run in Docker

```console
cd open-football
docker build -f .\build\Football.Dockerfile -t open-football .
docker run -d -p 18000:18000 --name open-football open-football

open chrome at http://localhost:18000
```


### License

Apache License 2.0