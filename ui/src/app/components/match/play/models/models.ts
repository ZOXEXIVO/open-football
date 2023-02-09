﻿import {Sprite} from "@pixi/sprite";
import {MatchLineupPlayerDto, ObjectPositionDto} from "../../services/match.api.service";
import {Graphics} from "pixi.js";

export class MatchModel {
  constructor() {
    this.players = [];
    this.ball = new BallModel([]);
    this.squad = new SquadModel();
  }

  public players: PlayerModel[];
  public ball: BallModel;
  public squad: SquadModel
}

export class PlayerModel {
  constructor(id: number, idHome: boolean, data: ObjectPositionDto[]) {
    this.id = id;
    this.isHome = idHome;
    this.obj = null;
    this.currentCoordIdx = 0;
    this.data = data;
  }

  public id: number;
  public isHome: boolean;
  public obj: Graphics | null;
  public currentCoordIdx: number;
  public data: ObjectPositionDto[];
}

export class BallModel {
  constructor(data: ObjectPositionDto[]) {
    this.obj = null;
    this.currentCoordIdx = 0;
    this.data = data;
  }

  public obj?: Sprite | null;
  public currentCoordIdx: number;
  public data: ObjectPositionDto[];
}

// Squad

export class SquadModel {
  constructor() {
    this.home = [];
    this.home_subs = [];

    this.away = [];
    this.away_subs = [];
  }

  public home: SquadPlayerModel[];
  public home_subs: SquadPlayerModel[];

  public away: SquadPlayerModel[];
  public away_subs: SquadPlayerModel[];
}

export class SquadPlayerModel {
  constructor(id: number,
              first_name: string, last_name: string, middle_name: string,
              position: string, team_slug: string) {
    this.id = id;
    this.first_name = first_name;
    this.last_name = last_name;
    this.middle_name = middle_name;
    this.position = position;
    this.team_slug = team_slug;
  }

  public id: number;
  public first_name: string;
  public last_name: string;
  public middle_name: string;
  public position: string;
  public team_slug: string;
}
