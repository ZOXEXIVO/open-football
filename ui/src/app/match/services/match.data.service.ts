import {Injectable} from "@angular/core";
import {MatchDataDto, MatchDto, ObjectPositionDto} from "./match.service";
import {Container} from "pixi.js";

@Injectable({
    providedIn: 'root',
})
export class MatchDataService {
    public match: MatchDto | null = null;
    public matchData: MatchDataDto | null = null;

    public width: number = 0;
    public height: number = 0;

    setMatch(match: MatchDto) {
        this.match = match;
    }

    setMatchData(data: MatchDataDto) {
        this.matchData = data;
    }

    setResolution(width: number, height: number){
        this.width = width;
        this.height = height;
    }

    refreshData(timestamp: number){
        let lastData = this.getData(timestamp);

        if(!lastData){
            return;
        }

        // update ball position
        if (lastData.ball) {
            const z = lastData.ball.position[2] || 0; // Get z-coordinate, default to 0
            let ballPosition = this.translateToField(lastData.ball.position[0], lastData.ball.position[1], z);

            this.match!.ball!.obj!.x = ballPosition.x;
            this.match!.ball!.obj!.y = ballPosition.y;

            // Apply z-coordinate visual effects
            // Scale ball based on height (perspective)
            const heightScale = 1.0 + Math.min(z / 15.0, 0.4);
            this.match!.ball!.obj!.scale.set(heightScale);

            // Adjust y-position for 3D height (isometric-like projection)
            const visualYOffset = z * 0.5;
            this.match!.ball!.obj!.y = ballPosition.y - visualYOffset;
        }

        // update players position
        this.match?.players.forEach(player => {
            let player_data = lastData.players.find((p) => p.id == player.id);

            if (player_data?.position) {
                let playerPosition = this.translateToField(player_data.position.position[0], player_data.position.position[1]);

                player.obj!.x = playerPosition.x;
                player.obj!.y = playerPosition.y;
            }
        });
    }

    translateToField(x: number, y: number, z: number = 0) {
        const real_field_width = this.width - 100;
        const real_field_height = this.height;

        const inner_field_width = 840;
        const inner_field_height = 545;

        // Define the offsets for the field boundaries
        const offsetX = 20; // Horizontal offset
        const offsetY = 70; // Vertical offset

        // Calculate the scaling factors
        const scale_x = (real_field_width - 2 * offsetX) / inner_field_width;
        const scale_y = (real_field_height - 2 * offsetY) / inner_field_height;

        // Apply the scaling and offsets to translate coordinates
        return {
            x: offsetX + 42 + x * scale_x,
            y: offsetY + y * scale_y - 10,
            z: z
        };
    }

    getData(timestamp: number): MatchResultData | null {
        // ball
        let ballData = this.matchData!.ball[this.match!.ball.currentCoordIdx];
        if(!ballData) {
            return null;
        }

        let ts = ballData.timestamp;

        while (ts < timestamp && this.match!.ball.currentCoordIdx < this.matchData!.ball.length) {
            const data = this.matchData!.ball[this.match!.ball.currentCoordIdx];
            if(!data) {
                return null;
            }
            ts = data.timestamp;
            this.match!.ball.currentCoordIdx++;
        }

        const ballResult = this.matchData!.ball[this.match!.ball.currentCoordIdx - 1];

        let players_results: PlayerDataResultModel[] = [];

        Object.entries(this.matchData?.players!).forEach(([key, value]: [string, ObjectPositionDto[]]) => {
            const player = this.match!.players.find((player) => player.id == Number(key))!;

            if(player){
                let dt = value![player.currentCoordIdx];
                if(dt) {
                    let pts = dt.timestamp;

                    while (pts < timestamp && player.currentCoordIdx < value!.length) {
                        dt = value![player.currentCoordIdx];

                        if(dt) {
                            pts = dt.timestamp;
                            player.currentCoordIdx++;
                        }
                    }

                    const playerPosition = value![player.currentCoordIdx];

                    players_results.push(new PlayerDataResultModel(player.id, playerPosition));
                }
            }
        });

        return new MatchResultData(players_results, ballResult);
    }

    setPlayerGraphicsObject(playerId: number, container: Container){
        const player = this.match!.players.find((player) => player.id == playerId);
        if(player) {
            player.obj = container;
        } else {
            console.error('player not found, playerId = ' + playerId);
        }
    }
}

export class MatchResultData {
    constructor(players: PlayerDataResultModel[], ball: ObjectPositionDto) {
        this.players = players;
        this.ball = ball;
    }

    public players: PlayerDataResultModel[];
    public ball: ObjectPositionDto;
}

export class PlayerDataResultModel {
    constructor(playerId: number, position: ObjectPositionDto) {
        this.id = playerId;
        this.position = position;
    }

    public id: number;
    public position: ObjectPositionDto;
}