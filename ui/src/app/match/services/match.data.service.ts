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

    // Track which chunks are loaded
    private loadedChunks: Set<number> = new Set();
    private chunkDurationMs: number = 0;

    setMatch(match: MatchDto) {
        this.match = match;
    }

    setMatchData(data: MatchDataDto) {
        this.matchData = data;
    }

    mergeMatchData(chunkData: MatchDataDto, chunkNumber: number) {
        if (!this.matchData) {
            this.matchData = chunkData;
            this.loadedChunks.add(chunkNumber);
            return;
        }

        // Merge ball data
        if (chunkData.ball && chunkData.ball.length > 0) {
            this.matchData.ball.push(...chunkData.ball);
        }

        // Merge player data
        if (chunkData.players) {
            Object.entries(chunkData.players).forEach(([key, value]: [string, ObjectPositionDto[]]) => {
                const playerId = Number(key);

                if (this.matchData!.players[playerId]) {
                    this.matchData!.players[playerId].push(...value);
                } else {
                    this.matchData!.players[playerId] = value;
                }
            });
        }

        this.loadedChunks.add(chunkNumber);
    }

    setChunkDuration(durationMs: number) {
        this.chunkDurationMs = durationMs;
    }

    getChunkNumberForTime(timestamp: number): number {
        if (this.chunkDurationMs === 0) return 0;
        return Math.floor(timestamp / this.chunkDurationMs);
    }

    isChunkLoaded(chunkNumber: number): boolean {
        return this.loadedChunks.has(chunkNumber);
    }

    reset() {
        this.loadedChunks.clear();
        this.chunkDurationMs = 0;
        this.matchData = null;
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

        // Clamp input coordinates to field bounds (0 to field dimensions)
        const clampedX = Math.max(0, Math.min(inner_field_width, x));
        const clampedY = Math.max(0, Math.min(inner_field_height, y));

        // Apply the scaling and offsets to translate coordinates
        return {
            x: offsetX + 42 + clampedX * scale_x,
            y: offsetY + clampedY * scale_y - 10,
            z: z
        };
    }

    getData(timestamp: number): MatchResultData | null {
        // Ensure ball index is within bounds
        if (this.match!.ball.currentCoordIdx >= this.matchData!.ball.length) {
            this.match!.ball.currentCoordIdx = this.matchData!.ball.length - 1;
        }
        if (this.match!.ball.currentCoordIdx < 0) {
            this.match!.ball.currentCoordIdx = 0;
        }

        // ball
        let ballData = this.matchData!.ball[this.match!.ball.currentCoordIdx];
        if(!ballData) {
            console.error('No ball data at index', this.match!.ball.currentCoordIdx);
            return null;
        }

        let ts = ballData.timestamp;

        // If seeking backward OR if current timestamp is way ahead, reset to beginning
        // This handles both backward seeks and chunk loading issues
        if (ts > timestamp || (timestamp - ts) > 60000) {
            console.log('Seeking backward or large gap detected: resetting ball index from', this.match!.ball.currentCoordIdx, 'to 0');
            this.match!.ball.currentCoordIdx = 0;
            ballData = this.matchData!.ball[0];
            ts = ballData.timestamp;
        }

        // Move forward to find the correct timestamp
        while (ts < timestamp && this.match!.ball.currentCoordIdx < this.matchData!.ball.length - 1) {
            this.match!.ball.currentCoordIdx++;
            const data = this.matchData!.ball[this.match!.ball.currentCoordIdx];
            if(!data) {
                console.warn('Missing ball data at index', this.match!.ball.currentCoordIdx);
                break;
            }
            ts = data.timestamp;
        }

        // Use current index, but ensure it's valid
        const ballIndex = Math.min(this.match!.ball.currentCoordIdx, this.matchData!.ball.length - 1);
        const ballResult = this.matchData!.ball[ballIndex];

        // Warn if we're showing data from a significantly different time
        if (Math.abs(ballResult.timestamp - timestamp) > 5000) {
            console.warn(`Ball position mismatch: requested ${timestamp}ms, showing ${ballResult.timestamp}ms (diff: ${Math.abs(ballResult.timestamp - timestamp)}ms)`);
        }

        let players_results: PlayerDataResultModel[] = [];

        Object.entries(this.matchData?.players!).forEach(([key, value]: [string, ObjectPositionDto[]]) => {
            const player = this.match!.players.find((player) => player.id == Number(key))!;

            if(player && value && value.length > 0){
                // Ensure player index is within bounds
                if (player.currentCoordIdx >= value.length) {
                    player.currentCoordIdx = value.length - 1;
                }
                if (player.currentCoordIdx < 0) {
                    player.currentCoordIdx = 0;
                }

                let dt = value[player.currentCoordIdx];
                if(dt) {
                    let pts = dt.timestamp;

                    // If seeking backward OR if current timestamp is way ahead, reset to beginning
                    // This handles both backward seeks and chunk loading issues
                    if (pts > timestamp || (timestamp - pts) > 60000) {
                        console.log('Seeking backward or large gap detected: resetting player', player.id, 'index from', player.currentCoordIdx, 'to 0');
                        player.currentCoordIdx = 0;
                        dt = value[0];
                        pts = dt.timestamp;
                    }

                    // Move forward to find the correct timestamp
                    while (pts < timestamp && player.currentCoordIdx < value.length - 1) {
                        player.currentCoordIdx++;
                        dt = value[player.currentCoordIdx];

                        if(dt) {
                            pts = dt.timestamp;
                        } else {
                            console.warn('Missing player data for player', player.id, 'at index', player.currentCoordIdx);
                            break;
                        }
                    }

                    // Use current index, but ensure it's valid
                    const playerIndex = Math.min(player.currentCoordIdx, value.length - 1);
                    const playerPosition = value[playerIndex];

                    if(playerPosition) {
                        // Warn if we're showing data from a significantly different time
                        if (Math.abs(playerPosition.timestamp - timestamp) > 5000) {
                            console.warn(`Player ${player.id} position mismatch: requested ${timestamp}ms, showing ${playerPosition.timestamp}ms`);
                        }
                        players_results.push(new PlayerDataResultModel(player.id, playerPosition));
                    }
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